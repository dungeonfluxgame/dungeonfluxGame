//! Private, test-only contract for the planned CustomerService boundary.
//!
//! This records the closed method and command alternatives without defining wire
//! messages, authenticating callers, or implementing a production handler. Export
//! scope remains the exact current df-auth grant; payment and entitlement facts do
//! not supply that grant. DataLifecycle storage and stream ownership remain open.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CustomerServiceMethod {
    Command,
    Inspect,
    GetOperation,
    Export,
}

const CUSTOMER_SERVICE_METHODS: [CustomerServiceMethod; 4] = [
    CustomerServiceMethod::Command,
    CustomerServiceMethod::Inspect,
    CustomerServiceMethod::GetOperation,
    CustomerServiceMethod::Export,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CustomerCommand {
    StartRecoveryChallenge,
    CompleteRecoveryChallenge,
    EstablishVerifiedAccount,
    LinkCurrentGuest,
    RecoverCredentials,
    RevokeSessions,
    AcceptTenantTransfer,
    InitiateCheckout,
    InitiatePortal,
    ChangeSubscription,
    CancelSubscription,
    RequestLifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequiredAuthority {
    PerCommandAuthorization,
    AnonymousChallenge,
    VerifiedAccount,
    TransferRecipientAcceptance,
    CurrentPayer,
    TenantLifecyclePermission,
    TrustedAccountOrTenant,
    CurrentExportGrant,
}

impl CustomerCommand {
    const fn required_authority(self) -> RequiredAuthority {
        match self {
            Self::StartRecoveryChallenge | Self::CompleteRecoveryChallenge => {
                RequiredAuthority::AnonymousChallenge
            }
            Self::EstablishVerifiedAccount
            | Self::LinkCurrentGuest
            | Self::RecoverCredentials
            | Self::RevokeSessions => RequiredAuthority::VerifiedAccount,
            Self::AcceptTenantTransfer => RequiredAuthority::TransferRecipientAcceptance,
            Self::InitiateCheckout
            | Self::InitiatePortal
            | Self::ChangeSubscription
            | Self::CancelSubscription => RequiredAuthority::CurrentPayer,
            Self::RequestLifecycle => RequiredAuthority::TenantLifecyclePermission,
        }
    }
}

impl CustomerServiceMethod {
    const fn required_authority(self) -> RequiredAuthority {
        match self {
            Self::Command => RequiredAuthority::PerCommandAuthorization,
            Self::Inspect | Self::GetOperation => RequiredAuthority::TrustedAccountOrTenant,
            Self::Export => RequiredAuthority::CurrentExportGrant,
        }
    }
}

struct ContractObservation {
    criterion: &'static str,
    decision: &'static str,
    alternative: &'static str,
    unresolved: &'static str,
}

const CONTRACT_OBSERVATIONS: [ContractObservation; 3] = [
    ContractObservation {
        criterion: "closed CustomerService method and command variants",
        decision: concat!(
            "Keep four methods and permit anonymous access only for recovery challenge ",
            "start/completion."
        ),
        alternative: "Do not add a generic tenant command or allow anonymous Inspect/Export.",
        unresolved: concat!(
            "Generated messages, field numbering, production authorization and response shapes ",
            "remain G03 work."
        ),
    },
    ContractObservation {
        criterion: "payment authority and secret export",
        decision: "Use the independent current df-auth ExportGrant and exact item/scope checks.",
        alternative: "Do not infer private export rights from payer or active subscription status.",
        unresolved: concat!(
            "The current ExportRequest has no recovery-epoch field; DataLifecycle source ",
            "selection is not implemented."
        ),
    },
    ContractObservation {
        criterion: "bounded export",
        decision: "Represent chunk, byte and item bounds as explicit caller-supplied limits.",
        alternative: "Do not invent numeric production limits or a durable streaming adapter.",
        unresolved: concat!(
            "DataLifecycle persistence, bounded stream ownership and numeric limits remain ",
            "unselected."
        ),
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExportBounds {
    chunks: u32,
    bytes: u64,
    items: u32,
}

impl ExportBounds {
    const fn admits(self, chunks: u32, bytes: u64, items: u32) -> bool {
        self.chunks > 0
            && self.bytes > 0
            && self.items > 0
            && chunks <= self.chunks
            && bytes <= self.bytes
            && items <= self.items
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CONTRACT_OBSERVATIONS, CUSTOMER_SERVICE_METHODS, CustomerCommand, CustomerServiceMethod,
        ExportBounds, RequiredAuthority,
    };

    #[test]
    fn customer_service_inventory_is_closed_to_four_planned_methods() {
        assert_eq!(
            CUSTOMER_SERVICE_METHODS,
            [
                CustomerServiceMethod::Command,
                CustomerServiceMethod::Inspect,
                CustomerServiceMethod::GetOperation,
                CustomerServiceMethod::Export,
            ]
        );
        assert_eq!(
            CUSTOMER_SERVICE_METHODS.map(CustomerServiceMethod::required_authority),
            [
                RequiredAuthority::PerCommandAuthorization,
                RequiredAuthority::TrustedAccountOrTenant,
                RequiredAuthority::TrustedAccountOrTenant,
                RequiredAuthority::CurrentExportGrant,
            ]
        );
    }

    #[test]
    fn anonymous_customer_commands_are_only_bounded_challenge_start_and_completion() {
        let commands = [
            CustomerCommand::StartRecoveryChallenge,
            CustomerCommand::CompleteRecoveryChallenge,
            CustomerCommand::EstablishVerifiedAccount,
            CustomerCommand::LinkCurrentGuest,
            CustomerCommand::RecoverCredentials,
            CustomerCommand::RevokeSessions,
            CustomerCommand::AcceptTenantTransfer,
            CustomerCommand::InitiateCheckout,
            CustomerCommand::InitiatePortal,
            CustomerCommand::ChangeSubscription,
            CustomerCommand::CancelSubscription,
            CustomerCommand::RequestLifecycle,
        ];
        let anonymous = commands
            .into_iter()
            .filter(|command| command.required_authority() == RequiredAuthority::AnonymousChallenge)
            .collect::<Vec<_>>();

        assert_eq!(
            anonymous,
            [
                CustomerCommand::StartRecoveryChallenge,
                CustomerCommand::CompleteRecoveryChallenge,
            ]
        );
        assert_eq!(
            CustomerCommand::RequestLifecycle.required_authority(),
            RequiredAuthority::TenantLifecyclePermission
        );
        assert_eq!(
            commands.map(CustomerCommand::required_authority),
            [
                RequiredAuthority::AnonymousChallenge,
                RequiredAuthority::AnonymousChallenge,
                RequiredAuthority::VerifiedAccount,
                RequiredAuthority::VerifiedAccount,
                RequiredAuthority::VerifiedAccount,
                RequiredAuthority::VerifiedAccount,
                RequiredAuthority::TransferRecipientAcceptance,
                RequiredAuthority::CurrentPayer,
                RequiredAuthority::CurrentPayer,
                RequiredAuthority::CurrentPayer,
                RequiredAuthority::CurrentPayer,
                RequiredAuthority::TenantLifecyclePermission,
            ]
        );
    }

    #[test]
    fn payment_commands_have_no_export_authority_and_export_bounds_are_explicit() {
        for command in [
            CustomerCommand::InitiateCheckout,
            CustomerCommand::InitiatePortal,
            CustomerCommand::ChangeSubscription,
            CustomerCommand::CancelSubscription,
        ] {
            assert_eq!(
                command.required_authority(),
                RequiredAuthority::CurrentPayer
            );
            assert_ne!(
                command.required_authority(),
                RequiredAuthority::CurrentExportGrant
            );
        }

        // Synthetic limits prove finite admission; the production values remain open.
        let bounds = ExportBounds {
            chunks: 2,
            bytes: 1024,
            items: 3,
        };
        assert!(bounds.admits(2, 1024, 3));
        assert!(!bounds.admits(3, 1, 1));
        assert!(!bounds.admits(1, 1025, 1));
        assert!(!bounds.admits(1, 1, 4));
        assert!(
            !(ExportBounds {
                chunks: 0,
                ..bounds
            })
            .admits(0, 1, 1)
        );
    }

    #[test]
    fn source_observations_keep_decisions_alternatives_and_open_adapters() {
        assert_eq!(CONTRACT_OBSERVATIONS.len(), 3);
        assert!(CONTRACT_OBSERVATIONS.iter().all(|observation| {
            !observation.criterion.is_empty()
                && !observation.decision.is_empty()
                && !observation.alternative.is_empty()
                && !observation.unresolved.is_empty()
        }));
    }
}
