//! Executable, native-only boundary witness for the D04 design decision.
//!
//! Lifecycle observations below are test-owned policy inputs. They do not verify a
//! gateway event, publish an entitlement, write a ledger, or acknowledge erasure.

#![cfg(not(target_arch = "wasm32"))]

use df_auth::membership::{
    MembershipAuthority, MembershipError, MembershipRecord, MembershipRequest,
    authorize_membership, revalidate_membership,
};
use df_auth::permissions::{
    AccessBasis, CurrentAccess, CurrentExport, CurrentOperator, ExportGrant, ExportItem,
    ExportRequest, OperatorGrant, OperatorPermission, OperatorRequest, PermissionAuthority,
    PermissionError, PermissionRefusal, PermissionScope, invoke_operator, publish_export,
};
use df_auth::rotation::CredentialFence;
use df_commerce::{
    Access, CommerceCommit, CommerceCommitOutcome, CommerceMutation, CommerceReadOutcome,
    CommerceRepository, CommerceScope, GatewayMutation, GatewayObservation, GatewayOperation,
    GatewayOperationKind, GatewayOperationOutcome, GatewayReconciliation, ObservationAuthority,
    PaidInvoice, PaymentGateway, PaymentOutcome, SpendConsent, SpendCounter,
    SpendOperationObservation, SpendOperationStatus, SpendRefusal, SpendRequest, SpendReservation,
    SpendReservationStatus, SpendSettlementObservation, SpendSettlementOutcome,
    SpendSettlementProposal, SpendSnapshot, SubscriptionObservation, SubscriptionRefusal,
    SubscriptionState, effective_access, observe_subscription, propose_spend,
    propose_spend_settlement,
};
use df_types::{
    Currency, LiabilityRate, Money, OperationId, PaidInvoiceId, SessionId, SubscriptionId, Usage,
    UsageUnit,
};
use std::time::Duration;

const PRINCIPAL: u64 = 2;
const PAYER: u64 = 3;
const TENANT: u64 = 4;
const CAMPAIGN: u64 = 5;
const CREDENTIAL: u64 = 6;
const GENERATION: u64 = 7;
const RECIPIENT: u64 = 8;
const AUDIENCE: u64 = 9;
const EXPORT_IDENTITY: u64 = 10;
const FACT: u64 = 11;
const SOURCE_VERSION: u64 = 12;
const ACCESS_VERSION: u64 = 13;
const RIGHTS_VERSION: u64 = 14;

type Scope = PermissionScope<u64, u64>;

fn scope() -> Scope {
    Scope {
        tenant: TENANT,
        campaign: CAMPAIGN,
        session: SessionId::from_bytes(&[15; 16]).unwrap(),
    }
}

fn currency() -> Currency {
    Currency::parse("USD").unwrap()
}

fn pending_subscription() -> SubscriptionState {
    SubscriptionState {
        currency: currency(),
        revision: 1,
        object_revision: 1,
        access: Access::PendingInitial,
        paid_through: 0,
        grace_until: None,
    }
}

fn settled_observation() -> SubscriptionObservation {
    SubscriptionObservation {
        authority: ObservationAuthority::VerifiedLatest,
        expected_revision: 1,
        object_revision: 2,
        outcome: PaymentOutcome::Settled(PaidInvoice {
            period_start: 90,
            paid_through: 200,
            selected_allowance: Money::new(currency(), 20),
            allowance_already_recorded: false,
        }),
    }
}

fn paid_subscription() -> SubscriptionState {
    observe_subscription(pending_subscription(), settled_observation(), 100)
        .unwrap()
        .next
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleObservation {
    CancelAtPeriodEnd,
    CancelImmediate,
    RefundConfirmedEndingEntitlement,
    RefundUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateReceipt {
    ProposedOnly,
    PendingReconciliation,
}

// This finite policy witness is deliberately private to the test. The native
// gateway, commerce transaction, journal, and disclosure owner remain absent.
fn governed_candidate(
    state: SubscriptionState,
    observation: LifecycleObservation,
) -> (SubscriptionState, CandidateReceipt) {
    match observation {
        LifecycleObservation::CancelAtPeriodEnd => (state, CandidateReceipt::ProposedOnly),
        LifecycleObservation::CancelImmediate
        | LifecycleObservation::RefundConfirmedEndingEntitlement => (
            SubscriptionState {
                access: Access::Restricted,
                ..state
            },
            CandidateReceipt::ProposedOnly,
        ),
        LifecycleObservation::RefundUnknown => (state, CandidateReceipt::PendingReconciliation),
    }
}

struct CurrentMembership {
    revision: u64,
    active: bool,
}

impl MembershipAuthority for CurrentMembership {
    type Principal = u64;
    type Tenant = u64;
    type Campaign = u64;
    type Role = u64;
    type Revision = u64;
    type Error = ();

    fn read_current(
        &mut self,
        _request: MembershipRequest<'_, Self>,
    ) -> Result<Option<MembershipRecord<Self>>, Self::Error> {
        Ok(Some(MembershipRecord {
            principal: PRINCIPAL,
            tenant: TENANT,
            campaign: CAMPAIGN,
            role: 1,
            revision: self.revision,
            active: self.active,
            expires_at: Some(200),
        }))
    }
}

struct CurrentPrivateAuthority {
    scope: Scope,
    credential_fence: CredentialFence<u64>,
    private_grant_present: bool,
    private_grant_active: bool,
    private_access_version: u64,
    consented: bool,
    source_rights: bool,
    export_enabled: bool,
    operator_permission: OperatorPermission,
    operator_grant_present: bool,
}

impl Default for CurrentPrivateAuthority {
    fn default() -> Self {
        Self {
            scope: scope(),
            credential_fence: CredentialFence::new(GENERATION),
            private_grant_present: false,
            private_grant_active: true,
            private_access_version: ACCESS_VERSION,
            consented: true,
            source_rights: true,
            export_enabled: true,
            operator_permission: OperatorPermission::Refund,
            operator_grant_present: false,
        }
    }
}

impl CurrentPrivateAuthority {
    fn access(&self) -> CurrentAccess<'_, Self> {
        CurrentAccess {
            principal: &PRINCIPAL,
            credential: &CREDENTIAL,
            fence: &self.credential_fence,
            credential_unexpired: true,
            binding_generation: &GENERATION,
            binding_active: true,
            binding_unexpired: true,
        }
    }
}

impl PermissionAuthority for CurrentPrivateAuthority {
    type Principal = u64;
    type Credential = u64;
    type CredentialGeneration = u64;
    type BindingGeneration = u64;
    type Tenant = u64;
    type Campaign = u64;
    type OperatorVersion = u64;
    type Recipient = u64;
    type Audience = u64;
    type ExportIdentity = u64;
    type Fact = u64;
    type Media = u64;
    type SourceVersion = u64;
    type AccessVersion = u64;
    type RightsVersion = u64;
    type Error = ();

    fn with_operator<R>(
        &mut self,
        _request: &OperatorRequest<'_, Self>,
        invoke: impl FnOnce(Option<CurrentOperator<'_, Self>>) -> R,
    ) -> Result<R, Self::Error> {
        Ok(invoke(self.operator_grant_present.then(|| {
            CurrentOperator {
                grant: OperatorGrant {
                    principal: &PRINCIPAL,
                    scope: &self.scope,
                    permission: self.operator_permission,
                    version: &1,
                    active: true,
                    issued_at_seconds: 0,
                    expires_at_seconds: 200,
                },
                access: self.access(),
                now_seconds: 100,
                independently_authenticated: true,
                mfa_verified: true,
                reason_ticket_verified: true,
                exceptional_audit_approved: false,
                private_rights_authorized: false,
                debug_enabled: false,
            }
        })))
    }

    fn with_export<R>(
        &mut self,
        _request: &ExportRequest<'_, Self>,
        publish: impl FnOnce(Option<CurrentExport<'_, Self>>) -> R,
    ) -> Result<R, Self::Error> {
        Ok(publish(self.private_grant_present.then(|| CurrentExport {
            grant: ExportGrant {
                principal: &PRINCIPAL,
                scope: &self.scope,
                recipient: &RECIPIENT,
                audience: &AUDIENCE,
                export_identity: &EXPORT_IDENTITY,
                permitted_item: ExportItem::Fact(&FACT),
                source_version: &SOURCE_VERSION,
                access_version: &self.private_access_version,
                rights_version: &RIGHTS_VERSION,
                active: self.private_grant_active,
                expires_at_seconds: 200,
                consented: self.consented,
                capture_contributor_rights: true,
                source_rights_and_attribution: self.source_rights,
            },
            access: self.access(),
            now_seconds: 100,
            export_enabled: self.export_enabled,
        })))
    }
}

fn access_basis() -> AccessBasis<'static, CurrentPrivateAuthority> {
    AccessBasis {
        credential: &CREDENTIAL,
        credential_generation: &GENERATION,
        binding_generation: &GENERATION,
    }
}

fn export_request(scope: &Scope) -> ExportRequest<'_, CurrentPrivateAuthority> {
    ExportRequest {
        principal: &PRINCIPAL,
        scope,
        recipient: &RECIPIENT,
        audience: &AUDIENCE,
        export_identity: &EXPORT_IDENTITY,
        item: ExportItem::Fact(&FACT),
        source_version: &SOURCE_VERSION,
        access_version: &ACCESS_VERSION,
        rights_version: &RIGHTS_VERSION,
        access: access_basis(),
    }
}

fn operator_request(
    scope: &Scope,
    permission: OperatorPermission,
) -> OperatorRequest<'_, CurrentPrivateAuthority> {
    OperatorRequest {
        principal: &PRINCIPAL,
        scope,
        permission,
        grant_version: &1,
        access: access_basis(),
    }
}

fn disclosed_bytes(
    authority: &mut CurrentPrivateAuthority,
    request: &ExportRequest<'_, CurrentPrivateAuthority>,
) -> (Result<(), PermissionError<(), ()>>, Vec<u8>) {
    let mut bytes = Vec::new();
    let result = publish_export(authority, request, || {
        bytes.extend_from_slice(b"private fact");
        Ok::<_, ()>(())
    });
    (result, bytes)
}

#[test]
fn verified_payment_and_current_membership_do_not_issue_private_export_grants() {
    let settled = observe_subscription(pending_subscription(), settled_observation(), 100).unwrap();
    assert_eq!(settled.next.access, Access::Active);
    assert_eq!(settled.allowance, Some(Money::new(currency(), 20)));
    assert_eq!(effective_access(settled.next, 100), Access::Active);

    let mut membership = CurrentMembership {
        revision: 1,
        active: true,
    };
    let checked = authorize_membership(
        &mut membership,
        MembershipRequest {
            principal: &PRINCIPAL,
            tenant: &TENANT,
            campaign: &CAMPAIGN,
            role: &1,
        },
        100,
    )
    .unwrap();
    assert_eq!(
        revalidate_membership(&mut membership, &checked, 100),
        Ok(())
    );

    let mut authority = CurrentPrivateAuthority::default();
    let requested_scope = scope();
    let request = export_request(&requested_scope);
    assert_eq!(
        disclosed_bytes(&mut authority, &request),
        (
            Err(PermissionError::Refused(PermissionRefusal::MissingGrant)),
            Vec::new()
        )
    );
    authority.private_grant_present = true;
    assert_eq!(
        disclosed_bytes(&mut authority, &request),
        (Ok(()), b"private fact".to_vec())
    );
    let payer_request = ExportRequest {
        principal: &PAYER,
        ..export_request(&requested_scope)
    };
    assert_eq!(
        disclosed_bytes(&mut authority, &payer_request).0,
        Err(PermissionError::Refused(PermissionRefusal::WrongPrincipal))
    );
}

#[test]
fn cancellation_and_refund_candidates_preserve_decisions_and_never_expand_private_rights() {
    let paid = paid_subscription();
    let committed_decision_revision = 42;
    let mut authority = CurrentPrivateAuthority {
        private_grant_present: true,
        ..CurrentPrivateAuthority::default()
    };
    let requested_scope = scope();
    let request = export_request(&requested_scope);

    let (period_end, receipt) = governed_candidate(paid, LifecycleObservation::CancelAtPeriodEnd);
    assert_eq!(receipt, CandidateReceipt::ProposedOnly);
    assert_eq!(effective_access(period_end, 199), Access::Active);
    assert_eq!(effective_access(period_end, 200), Access::ReadOnly);
    assert_eq!(committed_decision_revision, 42);

    for observation in [
        LifecycleObservation::CancelImmediate,
        LifecycleObservation::RefundConfirmedEndingEntitlement,
    ] {
        let (restricted, receipt) = governed_candidate(paid, observation);
        assert_eq!(receipt, CandidateReceipt::ProposedOnly);
        assert_eq!(effective_access(restricted, 100), Access::Restricted);
        assert_eq!(committed_decision_revision, 42);
        authority.private_grant_active = false;
        assert_eq!(
            disclosed_bytes(&mut authority, &request),
            (
                Err(PermissionError::Refused(PermissionRefusal::Revoked)),
                Vec::new()
            )
        );
    }
    let (unknown, receipt) = governed_candidate(paid, LifecycleObservation::RefundUnknown);
    assert_eq!(unknown, paid);
    assert_eq!(receipt, CandidateReceipt::PendingReconciliation);
    assert_eq!(
        observe_subscription(
            paid,
            SubscriptionObservation {
                authority: ObservationAuthority::VerifiedLatest,
                expected_revision: paid.revision,
                object_revision: paid.object_revision + 1,
                outcome: PaymentOutcome::Unknown,
            },
            100,
        ),
        Err(SubscriptionRefusal::UnknownPayment)
    );
}

#[test]
fn cancelled_or_unknown_payment_cannot_release_unknown_supplier_exposure() {
    let money = |micros| Money::new(currency(), micros);
    let operation = OperationId::from_bytes(&[17; 16]).unwrap();
    let snapshot = SpendSnapshot {
        currency: currency(),
        revision: 4,
        counters: [SpendCounter {
            used: money(90),
            limit: money(100),
        }; 6],
    };
    let request = SpendRequest {
        operation,
        expected_revision: 4,
        expected_consent_revision: 2,
        maximum_supplier_liability: money(5),
    };
    let consent = SpendConsent {
        revision: 2,
        maximum: money(5),
    };
    let (cancelled, _) =
        governed_candidate(paid_subscription(), LifecycleObservation::CancelImmediate);
    assert_eq!(cancelled.access, Access::Restricted);
    assert_eq!(
        propose_spend(
            &snapshot,
            consent,
            request,
            SpendOperationObservation {
                operation,
                status: SpendOperationStatus::Unknown,
            },
        ),
        Err(SpendRefusal::UnknownOperation)
    );
    assert_eq!(
        snapshot.counters.map(|counter| counter.used),
        [money(90); 6]
    );
}

#[test]
fn deletion_suppression_and_old_access_versions_cannot_reopen_private_export() {
    let paid = paid_subscription();
    let requested_scope = scope();
    let request = export_request(&requested_scope);
    let mut authority = CurrentPrivateAuthority {
        private_grant_present: true,
        ..CurrentPrivateAuthority::default()
    };
    assert_eq!(disclosed_bytes(&mut authority, &request).0, Ok(()));

    // A test-owned current suppression fact denies before physical cleanup.
    // No durable tombstone, backup restore, or deletion receipt is simulated.
    authority.private_grant_active = false;
    authority.private_access_version += 1;
    assert_eq!(effective_access(paid, 100), Access::Active);
    assert_eq!(
        disclosed_bytes(&mut authority, &request),
        (
            Err(PermissionError::Refused(PermissionRefusal::Revoked)),
            Vec::new()
        )
    );
    authority.private_grant_active = true;
    assert_eq!(
        disclosed_bytes(&mut authority, &request).0,
        Err(PermissionError::Refused(
            PermissionRefusal::StaleAccessVersion
        ))
    );
    authority.private_access_version = ACCESS_VERSION;
    authority.consented = false;
    assert_eq!(
        disclosed_bytes(&mut authority, &request).0,
        Err(PermissionError::Refused(PermissionRefusal::ConsentRequired))
    );
    authority.consented = true;
    authority.source_rights = false;
    assert_eq!(
        disclosed_bytes(&mut authority, &request).0,
        Err(PermissionError::Refused(PermissionRefusal::RightsRequired))
    );
}

#[test]
fn refund_and_deletion_operator_permissions_are_distinct_from_private_export() {
    let requested_scope = scope();
    let mut authority = CurrentPrivateAuthority {
        operator_grant_present: true,
        ..CurrentPrivateAuthority::default()
    };
    let mut handler_calls = 0;
    assert_eq!(
        invoke_operator(
            &mut authority,
            &operator_request(&requested_scope, OperatorPermission::Refund),
            || {
                handler_calls += 1;
                Ok::<_, ()>(())
            },
        ),
        Ok(())
    );
    assert_eq!(handler_calls, 1);
    assert_eq!(
        invoke_operator(
            &mut authority,
            &operator_request(&requested_scope, OperatorPermission::Deletion),
            || {
                handler_calls += 1;
                Ok::<_, ()>(())
            },
        ),
        Err(PermissionError::Refused(PermissionRefusal::WrongPermission))
    );
    assert_eq!(handler_calls, 1);
    assert_eq!(
        disclosed_bytes(&mut authority, &export_request(&requested_scope)).0,
        Err(PermissionError::Refused(PermissionRefusal::MissingGrant))
    );
}

#[test]
fn source_unavailability_and_revocation_remain_explicit_after_paid_observation() {
    let paid = paid_subscription();
    assert_eq!(effective_access(paid, 100), Access::Active);
    let mut membership = CurrentMembership {
        revision: 1,
        active: true,
    };
    let checked = authorize_membership(
        &mut membership,
        MembershipRequest {
            principal: &PRINCIPAL,
            tenant: &TENANT,
            campaign: &CAMPAIGN,
            role: &1,
        },
        100,
    )
    .unwrap();
    membership.revision = 2;
    assert_eq!(
        revalidate_membership(&mut membership, &checked, 100),
        Err(MembershipError::StaleRevision)
    );
    membership.active = false;
    assert_eq!(
        revalidate_membership(&mut membership, &checked, 100),
        Err(MembershipError::Revoked)
    );
    let mut authority = CurrentPrivateAuthority {
        private_grant_present: true,
        export_enabled: false,
        ..CurrentPrivateAuthority::default()
    };
    let requested_scope = scope();
    assert_eq!(
        disclosed_bytes(&mut authority, &export_request(&requested_scope)).0,
        Err(PermissionError::Refused(PermissionRefusal::ExportDisabled))
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortRefusal {
    WrongScope,
    ConflictingOperation,
    OperationPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortFailure {
    WrongOperation,
}

struct UnknownCommitRepository {
    account: u64,
    tenant: u64,
    payer: u64,
    campaign: u64,
    operation: Option<OperationId>,
    candidate: Option<CommerceCommit>,
    settlement_revision: Option<u64>,
    protected_journal: TestJournalHead,
}

impl CommerceRepository for UnknownCommitRepository {
    type AccountId = u64;
    type TenantId = u64;
    type PayerId = u64;
    type CampaignId = u64;
    type View = SubscriptionState;
    type Receipt = u64;
    type Refusal = PortRefusal;
    type Failure = PortFailure;

    fn read_current(
        &mut self,
        scope: CommerceScope<'_, u64, u64, u64, u64>,
        _deadline: Duration,
    ) -> CommerceReadOutcome<Self::View, Self::Refusal, Self::Failure> {
        if *scope.account == self.account
            && *scope.tenant == self.tenant
            && *scope.payer == self.payer
            && *scope.campaign == self.campaign
        {
            CommerceReadOutcome::Found(pending_subscription())
        } else {
            CommerceReadOutcome::Refused(PortRefusal::WrongScope)
        }
    }

    fn commit(
        &mut self,
        scope: CommerceScope<'_, u64, u64, u64, u64>,
        operation: OperationId,
        candidate: CommerceCommit,
        _deadline: Duration,
    ) -> CommerceMutation<Self::Receipt, Self::Refusal, Self::Failure> {
        if *scope.account != self.account
            || *scope.tenant != self.tenant
            || *scope.payer != self.payer
            || *scope.campaign != self.campaign
        {
            return CommerceMutation::Refused(PortRefusal::WrongScope);
        }
        if !test_journal_allows(
            self.protected_journal,
            TestProtectedEffect::EntitlementPublication,
        ) {
            return CommerceMutation::Pending;
        }
        if let CommerceCommit::SpendSettlement(proposal) = candidate
            && (proposal.operation != operation
                || self.settlement_revision != Some(proposal.before_revision)
                || Some(proposal.next.revision) != proposal.before_revision.checked_add(1))
        {
            return CommerceMutation::Refused(PortRefusal::ConflictingOperation);
        }
        if let Some(recorded) = self.operation {
            if recorded == operation && self.candidate == Some(candidate) {
                return CommerceMutation::Unknown;
            }
            return CommerceMutation::Refused(if recorded == operation {
                PortRefusal::ConflictingOperation
            } else {
                PortRefusal::OperationPending
            });
        }
        self.operation = Some(operation);
        self.candidate = Some(candidate);
        CommerceMutation::Unknown
    }

    fn reconcile(
        &mut self,
        scope: CommerceScope<'_, u64, u64, u64, u64>,
        operation: OperationId,
        _deadline: Duration,
    ) -> Result<CommerceCommitOutcome<Self::Receipt>, Self::Failure> {
        if *scope.account != self.account
            || *scope.tenant != self.tenant
            || *scope.payer != self.payer
            || *scope.campaign != self.campaign
            || self.operation != Some(operation)
        {
            return Err(PortFailure::WrongOperation);
        }
        Ok(CommerceCommitOutcome::Unknown)
    }
}

struct UnknownCancellationGateway {
    payer: u64,
    subscription: SubscriptionId,
    operation: Option<OperationId>,
    fingerprint: Option<u64>,
    kind: Option<GatewayOperationKind>,
    dispatches: u8,
}

impl PaymentGateway<u64> for UnknownCancellationGateway {
    type RequestFingerprint = u64;
    type Checkout = u64;
    type WebhookEvent = u64;
    type Receipt = u64;
    type Refusal = PortRefusal;
    type Failure = PortFailure;

    fn create_checkout(
        &mut self,
        payer: &u64,
        operation: OperationId,
        subscription: SubscriptionId,
        price_version: df_types::PriceVersion,
        customer_total: Money,
        fingerprint: &Self::RequestFingerprint,
        _deadline: Duration,
    ) -> GatewayMutation<Self::Checkout, Self::Refusal, Self::Failure> {
        if *payer != self.payer || subscription != self.subscription {
            return GatewayMutation::Refused(PortRefusal::WrongScope);
        }
        let kind = GatewayOperationKind::CreateCheckout {
            subscription,
            price_version,
            customer_total,
        };
        if let Some(recorded) = self.operation {
            if recorded == operation
                && self.fingerprint == Some(*fingerprint)
                && self.kind == Some(kind)
            {
                return GatewayMutation::Unknown;
            }
            return GatewayMutation::Refused(if recorded == operation {
                PortRefusal::ConflictingOperation
            } else {
                PortRefusal::OperationPending
            });
        }
        self.operation = Some(operation);
        self.fingerprint = Some(*fingerprint);
        self.kind = Some(kind);
        self.dispatches += 1;
        GatewayMutation::Unknown
    }

    fn observe_subscription(
        &mut self,
        payer: &u64,
        subscription: SubscriptionId,
        _deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure> {
        if *payer != self.payer || subscription != self.subscription {
            return Err(PortFailure::WrongOperation);
        }
        Ok(GatewayObservation::WebhookOnly)
    }

    fn verify_webhook(
        &mut self,
        _event: &Self::WebhookEvent,
        _deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure> {
        Ok(GatewayObservation::WebhookOnly)
    }

    fn inspect_invoice(
        &mut self,
        payer: &u64,
        invoice: PaidInvoiceId,
        _deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure> {
        if *payer != self.payer {
            return Err(PortFailure::WrongOperation);
        }
        Ok(GatewayObservation::VerifiedLatest(
            df_commerce::GatewayPaymentObservation {
                invoice: Some(invoice),
                object_revision: 1,
                outcome: PaymentOutcome::Unknown,
            },
        ))
    }

    fn apply(
        &mut self,
        operation: GatewayOperation<'_, u64>,
        fingerprint: &Self::RequestFingerprint,
        _deadline: Duration,
    ) -> GatewayMutation<Self::Receipt, Self::Refusal, Self::Failure> {
        if *operation.payer != self.payer {
            return GatewayMutation::Refused(PortRefusal::WrongScope);
        }
        if let Some(recorded) = self.operation {
            if recorded == operation.operation
                && self.fingerprint == Some(*fingerprint)
                && self.kind == Some(operation.kind)
            {
                return GatewayMutation::Unknown;
            }
            return GatewayMutation::Refused(if recorded == operation.operation {
                PortRefusal::ConflictingOperation
            } else {
                PortRefusal::OperationPending
            });
        }
        if operation.kind
            != (GatewayOperationKind::CancelSubscription {
                subscription: self.subscription,
                effective_at_period_end: true,
            })
        {
            return GatewayMutation::Refused(PortRefusal::WrongScope);
        }
        self.operation = Some(operation.operation);
        self.fingerprint = Some(*fingerprint);
        self.kind = Some(operation.kind);
        self.dispatches += 1;
        GatewayMutation::Unknown
    }

    fn reconcile(
        &mut self,
        payer: &u64,
        operation: OperationId,
        fingerprint: &Self::RequestFingerprint,
        _deadline: Duration,
    ) -> Result<GatewayReconciliation<Self::Receipt>, Self::Failure> {
        if *payer != self.payer
            || self.operation != Some(operation)
            || self.fingerprint != Some(*fingerprint)
        {
            return Err(PortFailure::WrongOperation);
        }
        Ok(GatewayReconciliation {
            operation,
            outcome: GatewayOperationOutcome::Unknown,
        })
    }
}

#[test]
fn repository_scope_and_ambiguous_grant_commit_require_same_key_reconciliation() {
    let operation = OperationId::from_bytes(&[31; 16]).unwrap();
    let account = 1;
    let tenant = TENANT;
    let payer = PAYER;
    let campaign = CAMPAIGN;
    let mut repository = UnknownCommitRepository {
        account,
        tenant,
        payer,
        campaign,
        operation: None,
        candidate: None,
        settlement_revision: None,
        protected_journal: TestJournalHead::Missing,
    };

    let wrong_account = account + 1;
    let wrong_tenant = tenant + 1;
    let wrong_payer = payer + 1;
    let wrong_campaign = campaign + 1;
    for (candidate_account, candidate_tenant, candidate_payer, candidate_campaign) in [
        (&wrong_account, &tenant, &payer, &campaign),
        (&account, &wrong_tenant, &payer, &campaign),
        (&account, &tenant, &wrong_payer, &campaign),
        (&account, &tenant, &payer, &wrong_campaign),
    ] {
        assert_eq!(
            repository.read_current(
                CommerceScope {
                    account: candidate_account,
                    tenant: candidate_tenant,
                    payer: candidate_payer,
                    campaign: candidate_campaign,
                },
                Duration::from_secs(2)
            ),
            CommerceReadOutcome::Refused(PortRefusal::WrongScope)
        );
        assert_eq!(
            repository.commit(
                CommerceScope {
                    account: candidate_account,
                    tenant: candidate_tenant,
                    payer: candidate_payer,
                    campaign: candidate_campaign,
                },
                operation,
                CommerceCommit::Subscription(
                    observe_subscription(pending_subscription(), settled_observation(), 100)
                        .unwrap(),
                ),
                Duration::from_secs(2),
            ),
            CommerceMutation::Refused(PortRefusal::WrongScope)
        );
    }
    assert_eq!(repository.operation, None);
    assert_eq!(repository.candidate, None);
    let candidate = CommerceCommit::Subscription(
        observe_subscription(pending_subscription(), settled_observation(), 100).unwrap(),
    );
    assert_eq!(
        repository.commit(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            operation,
            candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Pending
    );
    assert_eq!(repository.operation, None);
    assert_eq!(repository.candidate, None);
    repository.protected_journal = TestJournalHead::AuthenticatedCurrent {
        journal: 5,
        protected: 5,
    };
    assert_eq!(
        repository.commit(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            operation,
            candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Unknown
    );
    assert_eq!(repository.candidate, Some(candidate));
    assert_eq!(
        repository.commit(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            operation,
            candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Unknown
    );
    let conflicting_candidate = CommerceCommit::Subscription(
        observe_subscription(
            pending_subscription(),
            SubscriptionObservation {
                authority: ObservationAuthority::VerifiedLatest,
                expected_revision: 1,
                object_revision: 2,
                outcome: PaymentOutcome::InitialFailed,
            },
            100,
        )
        .unwrap(),
    );
    assert_eq!(
        repository.commit(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            operation,
            conflicting_candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Refused(PortRefusal::ConflictingOperation)
    );
    let different_operation = OperationId::from_bytes(&[35; 16]).unwrap();
    assert_eq!(
        repository.commit(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            different_operation,
            candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Refused(PortRefusal::OperationPending)
    );
    assert_eq!(repository.operation, Some(operation));
    assert_eq!(repository.candidate, Some(candidate));
    assert_eq!(
        repository.reconcile(
            CommerceScope {
                account: &account,
                tenant: &tenant,
                payer: &payer,
                campaign: &campaign,
            },
            operation,
            Duration::from_secs(2),
        ),
        Ok(CommerceCommitOutcome::Unknown)
    );
    assert_eq!(repository.candidate, Some(candidate));
}

#[test]
fn settlement_candidate_uses_the_same_scoped_repository_operation_and_revision() {
    let account = PRINCIPAL;
    let tenant = TENANT;
    let payer = PAYER;
    let campaign = CAMPAIGN;
    let operation = OperationId::from_bytes(&[76; 16]).unwrap();
    let amount = |micros| Money::new(currency(), micros);
    let snapshot = SpendSnapshot {
        currency: currency(),
        revision: 4,
        counters: [SpendCounter {
            used: amount(80),
            limit: amount(100),
        }; 6],
    };
    let observation = SpendSettlementObservation {
        authority: ObservationAuthority::VerifiedLatest,
        operation,
        expected_revision: 4,
        outcome: SpendSettlementOutcome::Known {
            actual_usage: Usage::new(35, UsageUnit::Token),
            billed_waste: amount(5),
            liability_rate: LiabilityRate::new(currency(), UsageUnit::Token, 1, 1).unwrap(),
        },
    };
    let proposal = propose_spend_settlement(
        &snapshot,
        SpendReservation {
            operation,
            maximum_supplier_liability: amount(60),
            status: SpendReservationStatus::UnknownLiability,
        },
        observation,
    )
    .unwrap();
    assert_eq!(proposal.operation, operation);
    assert_eq!(proposal.before_revision, snapshot.revision);
    assert_eq!(proposal.next.revision, 5);
    assert_eq!(snapshot.revision, 4);
    let candidate = CommerceCommit::SpendSettlement(proposal);
    let mut repository = UnknownCommitRepository {
        account,
        tenant,
        payer,
        campaign,
        operation: None,
        candidate: None,
        settlement_revision: Some(4),
        protected_journal: TestJournalHead::AuthenticatedCurrent {
            journal: 5,
            protected: 5,
        },
    };
    let scope = CommerceScope {
        account: &account,
        tenant: &tenant,
        payer: &payer,
        campaign: &campaign,
    };
    assert_eq!(
        repository.commit(scope, operation, candidate, Duration::from_secs(2)),
        CommerceMutation::Unknown
    );
    assert_eq!(repository.operation, Some(operation));
    assert_eq!(repository.candidate, Some(candidate));
    assert_eq!(
        repository.commit(scope, operation, candidate, Duration::from_secs(2)),
        CommerceMutation::Unknown
    );
    assert_eq!(
        repository.commit(
            scope,
            OperationId::from_bytes(&[77; 16]).unwrap(),
            candidate,
            Duration::from_secs(2),
        ),
        CommerceMutation::Refused(PortRefusal::ConflictingOperation)
    );
    let conflicting = CommerceCommit::SpendSettlement(SpendSettlementProposal {
        platform_loss: amount(1),
        ..proposal
    });
    assert_eq!(
        repository.commit(scope, operation, conflicting, Duration::from_secs(2)),
        CommerceMutation::Refused(PortRefusal::ConflictingOperation)
    );
    let stale = CommerceCommit::SpendSettlement(SpendSettlementProposal {
        before_revision: 3,
        ..proposal
    });
    assert_eq!(
        repository.commit(scope, operation, stale, Duration::from_secs(2)),
        CommerceMutation::Refused(PortRefusal::ConflictingOperation)
    );
    assert_eq!(
        repository.reconcile(scope, operation, Duration::from_secs(2)),
        Ok(CommerceCommitOutcome::Unknown)
    );
    assert_eq!(repository.candidate, Some(candidate));
}

#[test]
fn gateway_cancellation_unknown_and_webhook_observation_do_not_claim_success() {
    let payer = PAYER;
    let subscription = SubscriptionId::from_bytes(&[32; 16]).unwrap();
    let operation = OperationId::from_bytes(&[33; 16]).unwrap();
    let fingerprint = 991;
    let mut gateway = UnknownCancellationGateway {
        payer,
        subscription,
        operation: None,
        fingerprint: None,
        kind: None,
        dispatches: 0,
    };

    assert_eq!(
        gateway.observe_subscription(&payer, subscription, Duration::from_secs(2)),
        Ok(GatewayObservation::WebhookOnly)
    );
    assert_eq!(
        gateway.apply(
            GatewayOperation {
                payer: &payer,
                operation,
                kind: GatewayOperationKind::CancelSubscription {
                    subscription,
                    effective_at_period_end: true,
                },
            },
            &fingerprint,
            Duration::from_secs(2),
        ),
        GatewayMutation::Unknown
    );
    assert_eq!(
        gateway.reconcile(&payer, operation, &fingerprint, Duration::from_secs(2)),
        Ok(GatewayReconciliation {
            operation,
            outcome: GatewayOperationOutcome::Unknown,
        })
    );
    let conflicting_fingerprint = fingerprint + 1;
    assert_eq!(
        gateway.apply(
            GatewayOperation {
                payer: &payer,
                operation,
                kind: GatewayOperationKind::CancelSubscription {
                    subscription,
                    effective_at_period_end: false,
                },
            },
            &conflicting_fingerprint,
            Duration::from_secs(2),
        ),
        GatewayMutation::Refused(PortRefusal::ConflictingOperation)
    );
    let different_operation = OperationId::from_bytes(&[36; 16]).unwrap();
    assert_eq!(
        gateway.apply(
            GatewayOperation {
                payer: &payer,
                operation: different_operation,
                kind: GatewayOperationKind::CancelSubscription {
                    subscription,
                    effective_at_period_end: true,
                },
            },
            &fingerprint,
            Duration::from_secs(2),
        ),
        GatewayMutation::Refused(PortRefusal::OperationPending)
    );
    let wrong_payer = payer + 1;
    assert_eq!(
        gateway.reconcile(
            &wrong_payer,
            operation,
            &fingerprint,
            Duration::from_secs(2)
        ),
        Err(PortFailure::WrongOperation)
    );
    assert_eq!(gateway.dispatches, 1);
    assert_eq!(gateway.operation, Some(operation));
    assert_eq!(gateway.fingerprint, Some(fingerprint));
    assert_eq!(
        gateway.kind,
        Some(GatewayOperationKind::CancelSubscription {
            subscription,
            effective_at_period_end: true,
        })
    );
}

#[test]
fn checkout_deadline_fingerprint_and_supplier_identity_cannot_be_replaced() {
    let payer = PAYER;
    let subscription = SubscriptionId::from_bytes(&[37; 16]).unwrap();
    let price_version = df_types::PriceVersion::from_bytes(&[38; 16]).unwrap();
    let operation = OperationId::from_bytes(&[39; 16]).unwrap();
    let fingerprint = 1204;
    let total = Money::new(currency(), 123_000_000);
    let mut gateway = UnknownCancellationGateway {
        payer,
        subscription,
        operation: None,
        fingerprint: None,
        kind: None,
        dispatches: 0,
    };
    assert_eq!(
        gateway.create_checkout(
            &payer,
            operation,
            subscription,
            price_version,
            total,
            &fingerprint,
            Duration::from_secs(5),
        ),
        GatewayMutation::Unknown
    );
    assert_eq!(
        gateway.create_checkout(
            &payer,
            operation,
            subscription,
            price_version,
            total,
            &fingerprint,
            Duration::from_secs(5),
        ),
        GatewayMutation::Unknown
    );
    let altered_fingerprint = fingerprint + 1;
    assert_eq!(
        gateway.create_checkout(
            &payer,
            operation,
            subscription,
            price_version,
            total,
            &altered_fingerprint,
            Duration::from_secs(5),
        ),
        GatewayMutation::Refused(PortRefusal::ConflictingOperation)
    );
    assert_eq!(gateway.dispatches, 1);
    assert_eq!(gateway.operation, Some(operation));
    assert_eq!(gateway.fingerprint, Some(fingerprint));
    assert_eq!(
        gateway.kind,
        Some(GatewayOperationKind::CreateCheckout {
            subscription,
            price_version,
            customer_total: total,
        })
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestJournalHead {
    AuthenticatedCurrent { journal: u64, protected: u64 },
    Missing,
    Stale { journal: u64, protected: u64 },
    Conflicting { journal: u64, protected: u64 },
    Unauthenticated { journal: u64, protected: u64 },
    RestoredUnconfirmed { journal: u64, protected: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestProtectedEffect {
    EntitlementPublication,
    SupplierEgress,
    DeletionAcknowledgement,
}

// Test-owned Design facts only: no journal storage, cryptography, restore, or native
// acknowledgement is implemented here. Each irreversible effect needs the same
// authenticated, nonregressing head before its owner may acknowledge it.
fn test_journal_allows(head: TestJournalHead, _effect: TestProtectedEffect) -> bool {
    match head {
        TestJournalHead::AuthenticatedCurrent { journal, protected } => journal == protected,
        TestJournalHead::Missing => false,
        TestJournalHead::Stale { journal, protected }
        | TestJournalHead::Conflicting { journal, protected }
        | TestJournalHead::Unauthenticated { journal, protected }
        | TestJournalHead::RestoredUnconfirmed { journal, protected } => {
            let _observed_heads = (journal, protected);
            false
        }
    }
}

#[test]
fn missing_stale_conflicting_or_unauthenticated_journal_holds_all_protected_effects() {
    let effects = [
        TestProtectedEffect::EntitlementPublication,
        TestProtectedEffect::SupplierEgress,
        TestProtectedEffect::DeletionAcknowledgement,
    ];
    let held = [
        TestJournalHead::Missing,
        TestJournalHead::Stale {
            journal: 4,
            protected: 5,
        },
        TestJournalHead::Conflicting {
            journal: 6,
            protected: 5,
        },
        TestJournalHead::Unauthenticated {
            journal: 5,
            protected: 5,
        },
        TestJournalHead::RestoredUnconfirmed {
            journal: 5,
            protected: 5,
        },
    ];
    for head in held {
        for effect in effects {
            assert!(!test_journal_allows(head, effect));
        }
    }
    assert!(test_journal_allows(
        TestJournalHead::AuthenticatedCurrent {
            journal: 5,
            protected: 5,
        },
        TestProtectedEffect::EntitlementPublication,
    ));
}

// Include the crate's canonical policy contract cases in this one selected native
// consumer suite. This runs the actual existing sources without copying assertions or
// broadening the Cargo command to unrelated workspace targets.
#[path = "allowance.rs"]
mod canonical_allowance_contract_cases;
#[path = "credit.rs"]
mod canonical_credit_contract_cases;
#[path = "spend.rs"]
mod canonical_spend_contract_cases;
#[path = "spend_settlement.rs"]
mod canonical_spend_settlement_contract_cases;
#[path = "subscription.rs"]
mod canonical_subscription_contract_cases;

#[test]
fn exact_invoice_inspection_preserves_unknown_payment_as_nonsettlement() {
    let payer = PAYER;
    let subscription = SubscriptionId::from_bytes(&[32; 16]).unwrap();
    let invoice = PaidInvoiceId::from_bytes(&[34; 16]).unwrap();
    let mut gateway = UnknownCancellationGateway {
        payer,
        subscription,
        operation: None,
        fingerprint: None,
        kind: None,
        dispatches: 0,
    };
    let observed = gateway
        .inspect_invoice(&payer, invoice, Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        observed,
        GatewayObservation::VerifiedLatest(df_commerce::GatewayPaymentObservation {
            invoice: Some(invoice),
            object_revision: 1,
            outcome: PaymentOutcome::Unknown,
        })
    );
    assert_eq!(
        observe_subscription(
            pending_subscription(),
            SubscriptionObservation {
                authority: ObservationAuthority::VerifiedLatest,
                expected_revision: 1,
                object_revision: 2,
                outcome: PaymentOutcome::Unknown,
            },
            100,
        ),
        Err(SubscriptionRefusal::UnknownPayment)
    );
    assert_eq!(gateway.dispatches, 0);
}
