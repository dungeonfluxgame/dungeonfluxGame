//! The campaign role matrix of the master launch plan 3.2 (`planning/campaigns-and-table-research.md`
//! 4.1) as pure, table-driven policy.
//!
//! Owns what each campaign role may do and who may remove or readmit whom. It never reads a
//! store, never knows about money, and grants nothing by itself: the caller supplies the role and
//! delegations it read from the member row. Payment and campaign roles are independent, so no
//! role here carries a balance and no role change touches an allocation.

/// A campaign role. Support is operator access (`permissions.rs`) and is not a campaign role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CampaignRole {
    Host,
    CoHost,
    Player,
    Contributor,
    PairedDisplay,
}

impl CampaignRole {
    pub const ALL: [Self; 5] = [
        Self::Host,
        Self::CoHost,
        Self::Player,
        Self::Contributor,
        Self::PairedDisplay,
    ];

    /// Only the host and co-hosts may hold no seat (4.1); a seatless player cannot exist.
    pub const fn may_be_seatless(self) -> bool {
        matches!(self, Self::Host | Self::CoHost)
    }

    /// Rank for removal and readmission: the host outranks a co-host, which outranks the rest.
    const fn rank(self) -> u8 {
        match self {
            Self::Host => 3,
            Self::CoHost => 2,
            Self::Player | Self::Contributor | Self::PairedDisplay => 1,
        }
    }
}

/// One row of the plan 3.2 matrix.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    InviteOrRemovePlayers,
    StartOrPauseSession,
    AllocateOwnCredit,
    RaiseAnotherPayersExposure,
    ViewSharedPotTotals,
    ViewOwnCardAndReceipts,
    EditOwnCharacterTemplate,
    RewriteCampaignInventory,
    ViewPrivateCharacterNotes,
    ExportCampaign,
    DeleteCampaign,
}

impl Capability {
    pub const ALL: [Self; 11] = [
        Self::InviteOrRemovePlayers,
        Self::StartOrPauseSession,
        Self::AllocateOwnCredit,
        Self::RaiseAnotherPayersExposure,
        Self::ViewSharedPotTotals,
        Self::ViewOwnCardAndReceipts,
        Self::EditOwnCharacterTemplate,
        Self::RewriteCampaignInventory,
        Self::ViewPrivateCharacterNotes,
        Self::ExportCampaign,
        Self::DeleteCampaign,
    ];
}

/// One distinct cell wording of the plan 3.2 matrix, verbatim.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Grant {
    Yes,
    No,
    Never,
    Delegated,
    EmergencyPauseRequest,
    OptionalSummary,
    OwnOnly,
    ThroughLegalGameActions,
    AudienceFiltered,
    OwnPermittedView,
    FundingRecordsOnly,
    WithConfirmation,
    NotByDefault,
    LeaveOnly,
}

/// The plan 3.2 cell for a role and a capability.
pub const fn grant(role: CampaignRole, capability: Capability) -> Grant {
    use CampaignRole::{CoHost, Contributor, Host, PairedDisplay, Player};
    use Capability::{
        AllocateOwnCredit, DeleteCampaign, EditOwnCharacterTemplate, ExportCampaign,
        InviteOrRemovePlayers, RaiseAnotherPayersExposure, RewriteCampaignInventory,
        StartOrPauseSession, ViewOwnCardAndReceipts, ViewPrivateCharacterNotes,
        ViewSharedPotTotals,
    };
    match (capability, role) {
        (InviteOrRemovePlayers, Host) => Grant::Yes,
        (InviteOrRemovePlayers, CoHost) => Grant::Delegated,
        (InviteOrRemovePlayers, Player | Contributor | PairedDisplay) => Grant::No,

        (StartOrPauseSession, Host) => Grant::Yes,
        (StartOrPauseSession, CoHost) => Grant::Delegated,
        (StartOrPauseSession, Player) => Grant::EmergencyPauseRequest,
        (StartOrPauseSession, Contributor | PairedDisplay) => Grant::No,

        (AllocateOwnCredit, Host | CoHost | Player | Contributor) => Grant::Yes,
        (AllocateOwnCredit, PairedDisplay) => Grant::No,

        (RaiseAnotherPayersExposure, _) => Grant::No,

        (ViewSharedPotTotals, Host | CoHost | Player | Contributor) => Grant::Yes,
        (ViewSharedPotTotals, PairedDisplay) => Grant::OptionalSummary,

        (ViewOwnCardAndReceipts, Host | CoHost | Player | Contributor) => Grant::OwnOnly,
        (ViewOwnCardAndReceipts, PairedDisplay) => Grant::Never,

        (EditOwnCharacterTemplate, Host | CoHost | Player | Contributor) => Grant::Yes,
        (EditOwnCharacterTemplate, PairedDisplay) => Grant::No,

        (RewriteCampaignInventory, Host | CoHost | Player) => Grant::ThroughLegalGameActions,
        (RewriteCampaignInventory, Contributor | PairedDisplay) => Grant::No,

        (ViewPrivateCharacterNotes, Host | CoHost | Player | Contributor) => Grant::OwnOnly,
        (ViewPrivateCharacterNotes, PairedDisplay) => Grant::Never,

        (ExportCampaign, Host) => Grant::AudienceFiltered,
        (ExportCampaign, CoHost) => Grant::Delegated,
        (ExportCampaign, Player) => Grant::OwnPermittedView,
        (ExportCampaign, Contributor) => Grant::FundingRecordsOnly,
        (ExportCampaign, PairedDisplay) => Grant::No,

        (DeleteCampaign, Host) => Grant::WithConfirmation,
        (DeleteCampaign, CoHost) => Grant::NotByDefault,
        (DeleteCampaign, Player) => Grant::LeaveOnly,
        (DeleteCampaign, Contributor | PairedDisplay) => Grant::No,
    }
}

/// The three plan 3.2 cells that say "Delegated". There is no other delegation gate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Delegation {
    InvitePlayers,
    StartOrPauseSession,
    ExportCampaign,
}

impl Delegation {
    const fn bit(self) -> u8 {
        match self {
            Self::InvitePlayers => 1,
            Self::StartOrPauseSession => 2,
            Self::ExportCampaign => 4,
        }
    }
}

/// A co-host's delegations, set by the host.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Delegations(u8);

impl Delegations {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(7);

    /// Rejects a stored value with a bit this version does not know.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, delegation: Delegation) -> bool {
        self.0 & delegation.bit() != 0
    }

    #[must_use]
    pub const fn with(self, delegation: Delegation) -> Self {
        Self(self.0 | delegation.bit())
    }

    #[must_use]
    pub const fn without(self, delegation: Delegation) -> Self {
        Self(self.0 & !delegation.bit())
    }
}

/// How an export is filtered for the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Projection {
    AudienceFiltered,
    OwnPermittedView,
    FundingRecordsOnly,
}

/// What an allowed capability still obliges the caller to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authorized {
    Full,
    /// Only the caller's own object.
    OwnOnly,
    /// Only as a game command the rules accept; the role never edits state directly.
    ThroughLegalGameActions,
    Projection(Projection),
    /// The typed campaign-name confirmation (and, for delete, the owner layer) is still needed.
    RequiresConfirmation,
    /// A request for the host to act; never the action itself.
    RequestOnly,
    /// Allowed when the campaign enabled the display summary.
    WhenSummaryEnabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleRefusal {
    No,
    Never,
    NotDelegated,
    NotByDefault,
    /// A player may leave but not delete.
    LeaveOnly,
}

/// The delegation bit that gates a capability, or none. Exhaustive on purpose: a new capability
/// must say here which bit (if any) it needs, and a test pins every delegated cell to a bit.
pub const fn delegation_for(capability: Capability) -> Option<Delegation> {
    match capability {
        Capability::InviteOrRemovePlayers => Some(Delegation::InvitePlayers),
        Capability::StartOrPauseSession => Some(Delegation::StartOrPauseSession),
        Capability::ExportCampaign => Some(Delegation::ExportCampaign),
        Capability::AllocateOwnCredit
        | Capability::RaiseAnotherPayersExposure
        | Capability::ViewSharedPotTotals
        | Capability::ViewOwnCardAndReceipts
        | Capability::EditOwnCharacterTemplate
        | Capability::RewriteCampaignInventory
        | Capability::ViewPrivateCharacterNotes
        | Capability::DeleteCampaign => None,
    }
}

/// Turns a matrix cell into a decision, honouring the co-host's delegations.
pub fn authorize(
    role: CampaignRole,
    delegations: Delegations,
    capability: Capability,
) -> Result<Authorized, RoleRefusal> {
    match grant(role, capability) {
        Grant::Yes => Ok(Authorized::Full),
        Grant::No => Err(RoleRefusal::No),
        Grant::Never => Err(RoleRefusal::Never),
        Grant::NotByDefault => Err(RoleRefusal::NotByDefault),
        Grant::LeaveOnly => Err(RoleRefusal::LeaveOnly),
        Grant::Delegated => match delegation_for(capability) {
            Some(needed) if delegations.contains(needed) => Ok(Authorized::Full),
            Some(_) => Err(RoleRefusal::NotDelegated),
            // A delegated cell whose capability has no delegation bit is a table error; it never
            // grants.
            None => Err(RoleRefusal::No),
        },
        Grant::EmergencyPauseRequest => Ok(Authorized::RequestOnly),
        Grant::OptionalSummary => Ok(Authorized::WhenSummaryEnabled),
        Grant::OwnOnly => Ok(Authorized::OwnOnly),
        Grant::ThroughLegalGameActions => Ok(Authorized::ThroughLegalGameActions),
        Grant::AudienceFiltered => Ok(Authorized::Projection(Projection::AudienceFiltered)),
        Grant::OwnPermittedView => Ok(Authorized::Projection(Projection::OwnPermittedView)),
        Grant::FundingRecordsOnly => Ok(Authorized::Projection(Projection::FundingRecordsOnly)),
        Grant::WithConfirmation => Ok(Authorized::RequiresConfirmation),
    }
}

/// What a plan 3.2 "Support" cell permits. Support is an operator capability: this names the
/// ceiling so an operator surface cannot silently exceed it; `permissions.rs` still decides.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SupportAccess {
    AuditedAssistanceOnly,
    IncidentControl,
    NeverImpersonateConsent,
    Denied,
    ScopedAudited,
    RedactedSupportData,
    RecoveryAssistance,
    ExceptionalAuditedCorrection,
    BreakGlassJustified,
    BackupRoleSeparately,
    PolicyControlled,
}

/// The plan 3.2 Support column.
pub const fn support_grant(capability: Capability) -> SupportAccess {
    match capability {
        Capability::InviteOrRemovePlayers => SupportAccess::AuditedAssistanceOnly,
        Capability::StartOrPauseSession => SupportAccess::IncidentControl,
        Capability::AllocateOwnCredit => SupportAccess::NeverImpersonateConsent,
        Capability::RaiseAnotherPayersExposure => SupportAccess::Denied,
        Capability::ViewSharedPotTotals => SupportAccess::ScopedAudited,
        Capability::ViewOwnCardAndReceipts => SupportAccess::RedactedSupportData,
        Capability::EditOwnCharacterTemplate => SupportAccess::RecoveryAssistance,
        Capability::RewriteCampaignInventory => SupportAccess::ExceptionalAuditedCorrection,
        Capability::ViewPrivateCharacterNotes => SupportAccess::BreakGlassJustified,
        Capability::ExportCampaign => SupportAccess::BackupRoleSeparately,
        Capability::DeleteCampaign => SupportAccess::PolicyControlled,
    }
}

/// An owner right, separate from the host role (launch decision 11).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerAction {
    Archive,
    Delete,
    TransferOwnership,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerRefusal {
    NotOwner,
    /// The typed campaign name did not match.
    ConfirmationRequired,
}

/// Archive and delete need the owner and the typed campaign name; an ownership transfer needs the
/// owner only. The caller's current host role is irrelevant: a host who is not the owner is
/// refused, and an owner who is no longer the host is allowed.
pub const fn authorize_owner_action(
    is_owner: bool,
    action: OwnerAction,
    confirmation_matches: bool,
) -> Result<(), OwnerRefusal> {
    if !is_owner {
        return Err(OwnerRefusal::NotOwner);
    }
    match action {
        OwnerAction::TransferOwnership => Ok(()),
        OwnerAction::Archive | OwnerAction::Delete if confirmation_matches => Ok(()),
        OwnerAction::Archive | OwnerAction::Delete => Err(OwnerRefusal::ConfirmationRequired),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoveRefusal {
    /// The actor cannot invite or remove at all.
    NotAllowed,
    /// Nobody removes the host.
    TargetIsHost,
    /// The target is not below the actor.
    TargetOutranks,
    /// Nobody removes the owner (checked by the caller, which knows the owner).
    TargetIsOwner,
}

/// Whether `actor` may remove a member whose role is `target`. The caller passes whether the
/// target account owns the campaign: nobody removes the owner.
pub fn may_remove(
    actor: CampaignRole,
    actor_delegations: Delegations,
    target: CampaignRole,
    target_is_owner: bool,
) -> Result<(), RemoveRefusal> {
    if authorize(actor, actor_delegations, Capability::InviteOrRemovePlayers)
        != Ok(Authorized::Full)
    {
        return Err(RemoveRefusal::NotAllowed);
    }
    if target == CampaignRole::Host {
        return Err(RemoveRefusal::TargetIsHost);
    }
    if target_is_owner {
        return Err(RemoveRefusal::TargetIsOwner);
    }
    if actor.rank() <= target.rank() {
        return Err(RemoveRefusal::TargetOutranks);
    }
    Ok(())
}

/// Whether `actor` may allow back someone removed by a member whose role was `removed_by`: the
/// actor must be able to invite, and must rank the same as or higher than the remover.
pub fn may_allow_rejoin(
    actor: CampaignRole,
    actor_delegations: Delegations,
    removed_by: CampaignRole,
) -> bool {
    authorize(actor, actor_delegations, Capability::InviteOrRemovePlayers) == Ok(Authorized::Full)
        && actor.rank() >= removed_by.rank()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = include_str!("../../../planning/master-launch-plan.md");

    fn capability_of(label: &str) -> Capability {
        match label {
            "Invite/remove players" => Capability::InviteOrRemovePlayers,
            "Start/pause session" => Capability::StartOrPauseSession,
            "Allocate own credit" => Capability::AllocateOwnCredit,
            "Raise another payer's exposure" => Capability::RaiseAnotherPayersExposure,
            "View shared pot totals" => Capability::ViewSharedPotTotals,
            "View own card/receipts" => Capability::ViewOwnCardAndReceipts,
            "Edit own character template" => Capability::EditOwnCharacterTemplate,
            "Rewrite authoritative campaign inventory" => Capability::RewriteCampaignInventory,
            "View private character notes" => Capability::ViewPrivateCharacterNotes,
            "Export campaign" => Capability::ExportCampaign,
            "Delete campaign" => Capability::DeleteCampaign,
            other => panic!("the plan has a row this code does not know: {other:?}"),
        }
    }

    fn role_of(header: &str) -> Option<CampaignRole> {
        match header {
            "Host" => Some(CampaignRole::Host),
            "Co-host" => Some(CampaignRole::CoHost),
            "Player" => Some(CampaignRole::Player),
            "Contributor without seat" => Some(CampaignRole::Contributor),
            "Paired display" => Some(CampaignRole::PairedDisplay),
            "Support" => None,
            other => panic!("the plan has a column this code does not know: {other:?}"),
        }
    }

    fn grant_of(cell: &str) -> Grant {
        match cell {
            "Yes" => Grant::Yes,
            "No" => Grant::No,
            "Never" => Grant::Never,
            "Delegated" => Grant::Delegated,
            "Emergency pause request" => Grant::EmergencyPauseRequest,
            "Optional summary" => Grant::OptionalSummary,
            "Own only" => Grant::OwnOnly,
            "Through legal game actions" => Grant::ThroughLegalGameActions,
            "Audience-filtered" => Grant::AudienceFiltered,
            "Own permitted view" => Grant::OwnPermittedView,
            "Funding records only" => Grant::FundingRecordsOnly,
            "Host with confirmation" => Grant::WithConfirmation,
            "Not by default" => Grant::NotByDefault,
            "Leave only" => Grant::LeaveOnly,
            other => panic!("the plan has a cell this code does not know: {other:?}"),
        }
    }

    fn support_of(cell: &str) -> SupportAccess {
        match cell {
            "Audited assistance only" => SupportAccess::AuditedAssistanceOnly,
            "Incident control" => SupportAccess::IncidentControl,
            "Never impersonate consent" => SupportAccess::NeverImpersonateConsent,
            "No" => SupportAccess::Denied,
            "Scoped, audited" => SupportAccess::ScopedAudited,
            "Redacted support data" => SupportAccess::RedactedSupportData,
            "Recovery assistance" => SupportAccess::RecoveryAssistance,
            "Exceptional audited correction" => SupportAccess::ExceptionalAuditedCorrection,
            "Break-glass, justified" => SupportAccess::BreakGlassJustified,
            "Backup role separately" => SupportAccess::BackupRoleSeparately,
            "Policy-controlled" => SupportAccess::PolicyControlled,
            other => panic!("the plan has a support cell this code does not know: {other:?}"),
        }
    }

    fn matrix_rows() -> Vec<Vec<&'static str>> {
        let start = PLAN
            .find("## 3.2 Role matrix")
            .expect("the plan has section 3.2");
        PLAN[start..]
            .lines()
            .skip(1)
            .skip_while(|line| !line.starts_with('|'))
            .take_while(|line| line.starts_with('|'))
            .map(|line| {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(str::trim)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn every_cell_of_plan_3_2_matches_grant() {
        let rows = matrix_rows();
        assert_eq!(
            rows.len(),
            2 + Capability::ALL.len(),
            "header, separator and 11 rows"
        );
        let headers = &rows[0];
        assert_eq!(headers.len(), 7);
        let mut checked = 0;
        for row in &rows[2..] {
            let capability = capability_of(row[0]);
            for (index, cell) in row.iter().enumerate().skip(1) {
                match role_of(headers[index]) {
                    Some(role) => {
                        assert_eq!(
                            grant(role, capability),
                            grant_of(cell),
                            "{role:?} {capability:?}"
                        );
                    }
                    None => assert_eq!(
                        support_grant(capability),
                        support_of(cell),
                        "{capability:?}"
                    ),
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 11 * 6, "55 role cells and 11 support cells");
    }

    #[test]
    fn delegated_cells_need_exactly_their_own_bit() {
        for (capability, bit) in [
            (Capability::InviteOrRemovePlayers, Delegation::InvitePlayers),
            (
                Capability::StartOrPauseSession,
                Delegation::StartOrPauseSession,
            ),
            (Capability::ExportCampaign, Delegation::ExportCampaign),
        ] {
            assert_eq!(
                authorize(CampaignRole::CoHost, Delegations::NONE, capability),
                Err(RoleRefusal::NotDelegated)
            );
            assert_eq!(
                authorize(
                    CampaignRole::CoHost,
                    Delegations::NONE.with(bit),
                    capability
                ),
                Ok(Authorized::Full)
            );
            for other in [
                Delegation::InvitePlayers,
                Delegation::StartOrPauseSession,
                Delegation::ExportCampaign,
            ] {
                if other != bit {
                    assert_eq!(
                        authorize(
                            CampaignRole::CoHost,
                            Delegations::ALL.without(bit).with(other),
                            capability
                        ),
                        Err(RoleRefusal::NotDelegated)
                    );
                }
            }
        }
    }

    #[test]
    fn a_host_needs_no_delegation_and_a_player_cannot_be_delegated_anything() {
        assert_eq!(
            authorize(
                CampaignRole::Host,
                Delegations::NONE,
                Capability::InviteOrRemovePlayers
            ),
            Ok(Authorized::Full)
        );
        assert_eq!(
            authorize(
                CampaignRole::Player,
                Delegations::ALL,
                Capability::InviteOrRemovePlayers
            ),
            Err(RoleRefusal::No)
        );
    }

    #[test]
    fn delegations_reject_unknown_bits_and_round_trip() {
        assert_eq!(Delegations::from_bits(8), None);
        assert_eq!(Delegations::from_bits(7), Some(Delegations::ALL));
        let some = Delegations::NONE.with(Delegation::ExportCampaign);
        assert_eq!(Delegations::from_bits(some.bits()), Some(some));
        assert!(!some.contains(Delegation::InvitePlayers));
    }

    #[test]
    fn money_capabilities_belong_to_every_seated_or_seatless_member_and_never_to_a_display() {
        for role in [
            CampaignRole::Host,
            CampaignRole::CoHost,
            CampaignRole::Player,
            CampaignRole::Contributor,
        ] {
            assert_eq!(grant(role, Capability::AllocateOwnCredit), Grant::Yes);
        }
        for role in CampaignRole::ALL {
            assert_eq!(
                grant(role, Capability::RaiseAnotherPayersExposure),
                Grant::No
            );
        }
        assert_eq!(
            grant(CampaignRole::PairedDisplay, Capability::AllocateOwnCredit),
            Grant::No
        );
    }

    #[test]
    fn a_player_may_leave_but_not_delete_and_a_host_needs_the_typed_name() {
        assert_eq!(
            authorize(
                CampaignRole::Player,
                Delegations::NONE,
                Capability::DeleteCampaign
            ),
            Err(RoleRefusal::LeaveOnly)
        );
        assert_eq!(
            authorize(
                CampaignRole::Host,
                Delegations::NONE,
                Capability::DeleteCampaign
            ),
            Ok(Authorized::RequiresConfirmation)
        );
    }

    #[test]
    fn owner_actions_need_the_owner_and_a_host_who_is_not_the_owner_is_refused() {
        assert_eq!(
            authorize_owner_action(false, OwnerAction::Delete, true),
            Err(OwnerRefusal::NotOwner)
        );
        assert_eq!(
            authorize_owner_action(true, OwnerAction::Delete, false),
            Err(OwnerRefusal::ConfirmationRequired)
        );
        assert_eq!(
            authorize_owner_action(true, OwnerAction::Archive, true),
            Ok(())
        );
        assert_eq!(
            authorize_owner_action(true, OwnerAction::TransferOwnership, false),
            Ok(())
        );
    }

    #[test]
    fn only_the_host_and_cohosts_may_be_seatless() {
        assert!(CampaignRole::Host.may_be_seatless());
        assert!(CampaignRole::CoHost.may_be_seatless());
        assert!(!CampaignRole::Player.may_be_seatless());
        assert!(!CampaignRole::Contributor.may_be_seatless());
    }

    #[test]
    fn the_rank_table_decides_who_removes_whom() {
        let invite = Delegations::NONE.with(Delegation::InvitePlayers);
        let host = CampaignRole::Host;
        let cohost = CampaignRole::CoHost;
        assert_eq!(may_remove(host, Delegations::NONE, cohost, false), Ok(()));
        assert_eq!(
            may_remove(host, Delegations::NONE, CampaignRole::Player, false),
            Ok(())
        );
        assert_eq!(
            may_remove(host, Delegations::NONE, CampaignRole::Contributor, false),
            Ok(())
        );
        assert_eq!(
            may_remove(cohost, invite, CampaignRole::Player, false),
            Ok(())
        );
        assert_eq!(
            may_remove(cohost, invite, CampaignRole::Contributor, false),
            Ok(())
        );
        assert_eq!(
            may_remove(cohost, invite, cohost, false),
            Err(RemoveRefusal::TargetOutranks)
        );
        assert_eq!(
            may_remove(cohost, invite, host, false),
            Err(RemoveRefusal::TargetIsHost)
        );
        assert_eq!(
            may_remove(cohost, Delegations::NONE, CampaignRole::Player, false),
            Err(RemoveRefusal::NotAllowed)
        );
        assert_eq!(
            may_remove(
                CampaignRole::Player,
                Delegations::ALL,
                CampaignRole::Player,
                false
            ),
            Err(RemoveRefusal::NotAllowed)
        );
    }

    #[test]
    fn nobody_can_remove_the_host_or_the_owner() {
        assert_eq!(
            may_remove(
                CampaignRole::Host,
                Delegations::ALL,
                CampaignRole::Host,
                false
            ),
            Err(RemoveRefusal::TargetIsHost)
        );
        for target in [
            CampaignRole::CoHost,
            CampaignRole::Player,
            CampaignRole::Contributor,
        ] {
            assert_eq!(
                may_remove(CampaignRole::Host, Delegations::ALL, target, true),
                Err(RemoveRefusal::TargetIsOwner)
            );
        }
    }

    #[test]
    fn only_the_removers_rank_or_higher_may_allow_a_rejoin() {
        let invite = Delegations::NONE.with(Delegation::InvitePlayers);
        assert!(may_allow_rejoin(
            CampaignRole::Host,
            Delegations::NONE,
            CampaignRole::Host
        ));
        assert!(may_allow_rejoin(
            CampaignRole::Host,
            Delegations::NONE,
            CampaignRole::CoHost
        ));
        assert!(may_allow_rejoin(
            CampaignRole::CoHost,
            invite,
            CampaignRole::CoHost
        ));
        assert!(!may_allow_rejoin(
            CampaignRole::CoHost,
            invite,
            CampaignRole::Host
        ));
        assert!(!may_allow_rejoin(
            CampaignRole::CoHost,
            Delegations::NONE,
            CampaignRole::CoHost
        ));
        assert!(!may_allow_rejoin(
            CampaignRole::Player,
            Delegations::ALL,
            CampaignRole::CoHost
        ));
    }

    #[test]
    fn every_delegated_cell_has_a_delegation_bit_and_no_other_capability_has_one() {
        for role in CampaignRole::ALL {
            for capability in Capability::ALL {
                let delegated = grant(role, capability) == Grant::Delegated;
                if delegated {
                    assert!(
                        delegation_for(capability).is_some(),
                        "{role:?} {capability:?}"
                    );
                }
            }
        }
        let with_bits: Vec<_> = Capability::ALL
            .into_iter()
            .filter(|capability| delegation_for(*capability).is_some())
            .collect();
        assert_eq!(
            with_bits,
            [
                Capability::InviteOrRemovePlayers,
                Capability::StartOrPauseSession,
                Capability::ExportCampaign
            ]
        );
    }
}
