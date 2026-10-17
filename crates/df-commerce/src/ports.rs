//! Consumer-owned native boundaries around the existing pure commercial policies.
//!
//! These traits define requests and outcomes only. They do not authenticate a scope,
//! verify a supplier response, persist a ledger, confirm a protected journal, or send
//! provider traffic. Native owners must bind every call to current `df-auth` authority
//! and the single commerce ledger. In particular, a request identity is never proof
//! of account, tenant, payer, campaign, rights, consent, or operator permission.

use std::time::Duration;

use df_types::{Money, OperationId, PaidInvoiceId, PriceVersion, SubscriptionId};

use crate::{
    AllowanceTransition, CreditTransition, PaymentOutcome, SpendProposal, SpendSettlementProposal,
    SubscriptionTransition,
};

/// Identity tuple supplied by a native owner after authentication, used only to bind a
/// repository lookup or commit. The referenced values remain generic because these
/// roles do not have canonical `df-types` IDs; this tuple does not mint their authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommerceScope<'a, AccountId, TenantId, PayerId, CampaignId> {
    pub account: &'a AccountId,
    pub tenant: &'a TenantId,
    pub payer: &'a PayerId,
    pub campaign: &'a CampaignId,
}

/// One already-proposed pure policy candidate. A candidate is not a durable mutation
/// or permission to dispatch. The repository must revalidate its source revisions and
/// identities while atomically committing all corresponding ledger/grant/reservation
/// rows and the protected recovery-journal boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommerceCommit {
    Subscription(SubscriptionTransition),
    Allowance(AllowanceTransition),
    Credit(CreditTransition),
    Spend(SpendProposal),
    SpendSettlement(SpendSettlementProposal),
}

/// Result of a durable commerce mutation. `Committed` is the only success. `Unknown`
/// means the commit may have happened and must retain maximum liability until the same
/// `OperationId` is reconciled. Cancellation, timeout, restore, or owner loss cannot
/// convert it into `NotCommitted`. `Failed` is reserved for a failure known to have
/// made no durable change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommerceMutation<Receipt, Refusal, Failure> {
    Committed { receipt: Receipt, replayed: bool },
    Refused(Refusal),
    Pending,
    Unknown,
    Failed(Failure),
}

/// Read-back of the same operation after a timeout, lost acknowledgement, or restore.
/// `NotCommitted` is valid only after complete same-key and protected-journal lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommerceCommitOutcome<Receipt> {
    Committed { receipt: Receipt, replayed: bool },
    NotCommitted,
    Pending,
    Unknown,
}

/// Result of an authorized read, keeping absence, scope refusal, and storage failure
/// distinct so callers never infer that a request-supplied identity established scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommerceReadOutcome<View, Refusal, Failure> {
    Found(View),
    Missing,
    Refused(Refusal),
    Failed(Failure),
}

/// Native implementation of the one commerce repository/ledger authority.
///
/// Associated IDs are supplied by their existing owners. Implementations must derive
/// the scope from current `df-auth` state, not request fields; enforce account/tenant/
/// payer/campaign and current grant/consent/rights; compare every policy revision;
/// commit allowance, spend, and ledger effects once; and hold publication, egress, and
/// deletion acknowledgement unless the protected journal is current and confirmed.
/// Do not hold repository locks while calling `PaymentGateway`.
pub trait CommerceRepository {
    type AccountId;
    type TenantId;
    type PayerId;
    type CampaignId;
    type View;
    type Receipt;
    type Refusal;
    type Failure;

    /// Reads a scoped policy view after native authorization. Cached views are not
    /// admission authority and do not authorize paid work by themselves.
    fn read_current(
        &mut self,
        scope: CommerceScope<'_, Self::AccountId, Self::TenantId, Self::PayerId, Self::CampaignId>,
        deadline: Duration,
    ) -> CommerceReadOutcome<Self::View, Self::Refusal, Self::Failure>;

    /// Revalidates and atomically commits one policy candidate. The operation key is
    /// stable across retries; an ambiguous return remains `Unknown` for same-key
    /// reconciliation and must never cause a new reservation or grant.
    fn commit(
        &mut self,
        scope: CommerceScope<'_, Self::AccountId, Self::TenantId, Self::PayerId, Self::CampaignId>,
        operation: OperationId,
        candidate: CommerceCommit,
        deadline: Duration,
    ) -> CommerceMutation<Self::Receipt, Self::Refusal, Self::Failure>;

    /// Resolves the exact operation against complete durable and protected-journal
    /// evidence. Expiry or a missing cache entry is not proof that an operation is new.
    fn reconcile(
        &mut self,
        scope: CommerceScope<'_, Self::AccountId, Self::TenantId, Self::PayerId, Self::CampaignId>,
        operation: OperationId,
        deadline: Duration,
    ) -> Result<CommerceCommitOutcome<Self::Receipt>, Self::Failure>;
}

/// Gateway mutation whose identity is stable across retries and reconciliation. The
/// fingerprint is supplied by the owning service and must bind the exact request bytes
/// and selected commercial terms; changing it under one operation key is a conflict.
/// The request deliberately excludes credentials, card data, and private customer
/// payloads; native adapters acquire secrets separately and never return them here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayOperation<'a, PayerId> {
    pub payer: &'a PayerId,
    pub operation: OperationId,
    pub kind: GatewayOperationKind,
}

/// Supplier-facing actions supported by this narrow contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayOperationKind {
    CreateCheckout {
        subscription: SubscriptionId,
        price_version: PriceVersion,
        customer_total: Money,
    },
    CancelSubscription {
        subscription: SubscriptionId,
        effective_at_period_end: bool,
    },
    Refund {
        invoice: PaidInvoiceId,
        amount: Money,
    },
}

/// A supplier observation is input to pure policy only. `VerifiedLatest` is a
/// provenance assertion for the native verifier, not proof supplied by this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayPaymentObservation {
    pub invoice: Option<PaidInvoiceId>,
    pub object_revision: u64,
    pub outcome: PaymentOutcome,
}

/// Supplier mutation outcome. Only a known completed result may be acknowledged;
/// pending and unknown outcomes retain the operation and any maximum liability.
/// `Failed` is valid only when the adapter knows the supplier did not apply the action.
/// A timeout after a possible send must be `Unknown`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayMutation<Receipt, Refusal, Failure> {
    Completed { receipt: Receipt, replayed: bool },
    Refused(Refusal),
    Pending,
    Unknown,
    Failed(Failure),
}

/// Reconciliation facts for one gateway operation; absence is not proof of failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayOperationOutcome<Receipt> {
    Completed { receipt: Receipt, replayed: bool },
    KnownNotApplied,
    Pending,
    Unknown,
}

/// Native payment-gateway port. Adapters own supplier-specific verification and
/// credential handling. They must use the supplied operation identity and exact
/// request fingerprint for mutation and reconciliation, perform no network I/O under
/// repository locks, and treat a timeout after possible send as `Unknown` until
/// conclusive same-key lookup. Every call receives a finite caller-selected deadline;
/// implementations must stop owned streams/work at expiry while preserving uncertainty
/// for any external action that may already have started.
pub trait PaymentGateway<PayerId> {
    type RequestFingerprint;
    type Checkout;
    type WebhookEvent;
    type Receipt;
    type Refusal;
    type Failure;

    /// Creates checkout using the same operation/fingerprint on retry and an explicit
    /// native deadline. Return URLs and customer-facing copy must already be selected
    /// by an allowlisted native owner; arrival at a return URL is not payment evidence.
    #[allow(
        clippy::too_many_arguments,
        reason = "the payment gateway contract keeps each validated checkout field explicit"
    )]
    fn create_checkout(
        &mut self,
        payer: &PayerId,
        operation: OperationId,
        subscription: SubscriptionId,
        price_version: PriceVersion,
        customer_total: Money,
        fingerprint: &Self::RequestFingerprint,
        deadline: Duration,
    ) -> GatewayMutation<Self::Checkout, Self::Refusal, Self::Failure>;

    /// Refreshes an authoritative supplier object. Webhook-only or unverified events
    /// must not be represented as a verified latest observation by the adapter.
    fn observe_subscription(
        &mut self,
        payer: &PayerId,
        subscription: SubscriptionId,
        deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure>;

    /// Verifies the supplied opaque native event input within the deadline. A verified
    /// webhook may trigger an authoritative refresh but remains webhook-only evidence
    /// until that refresh completes; unknown fields/payloads must not enter policy logs.
    fn verify_webhook(
        &mut self,
        event: &Self::WebhookEvent,
        deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure>;

    /// Looks up one exact invoice through the supplier's authoritative object API.
    fn inspect_invoice(
        &mut self,
        payer: &PayerId,
        invoice: PaidInvoiceId,
        deadline: Duration,
    ) -> Result<GatewayObservation, Self::Failure>;

    /// Applies cancellation or refund with a stable operation identity. A lost response
    /// after possible send is `Unknown`, never an inferred success or safe retry.
    fn apply(
        &mut self,
        operation: GatewayOperation<'_, PayerId>,
        fingerprint: &Self::RequestFingerprint,
        deadline: Duration,
    ) -> GatewayMutation<Self::Receipt, Self::Refusal, Self::Failure>;

    /// Queries the same operation identity; never substitutes a new provider key.
    fn reconcile(
        &mut self,
        payer: &PayerId,
        operation: OperationId,
        fingerprint: &Self::RequestFingerprint,
        deadline: Duration,
    ) -> Result<GatewayReconciliation<Self::Receipt>, Self::Failure>;
}

/// Latest-object provenance and policy outcome as observed by an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayObservation {
    VerifiedLatest(GatewayPaymentObservation),
    WebhookOnly,
    Unverified,
}

/// Same-key gateway lookup result. `KnownNotApplied` must be supported by conclusive
/// supplier evidence; unsupported status lookup remains `Unknown`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayReconciliation<Receipt> {
    pub operation: OperationId,
    pub outcome: GatewayOperationOutcome<Receipt>,
}
