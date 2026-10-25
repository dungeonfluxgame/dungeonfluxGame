//! Native account API adapters; missing owner contracts refuse without changing state.

use df_account_app::wire::{
    ActionOutcome, AssetKind, AssetsView, AuditRow, EmailPreferences, ExportStatus, ReviewDecision,
    ReviewItem,
};
use df_portal::app_api::ports::*;
use df_types::{AccountId, OperationId};

pub(crate) struct Refusing;

// TODO(B): operator capabilities are denied until the PG grant adapter is exported.
impl OperatorDirectory for Refusing {
    async fn grant(&self, _account: AccountId) -> PortResult<Option<OperatorGrant>> {
        Ok(None)
    }

    async fn append_audit(&self, _entry: AuditEntry) -> PortResult<String> {
        unavailable("B")
    }

    async fn audit(&self) -> PortResult<Vec<AuditRow>> {
        unavailable("B")
    }

    async fn revoke_account_session(
        &self,
        _act: OperatorAct,
        _account: AccountId,
        _session_id: &str,
    ) -> PortResult<ActionOutcome> {
        unavailable("B")
    }
}

// TODO(O2): the review owner must export its PG queue projection.
impl ReviewQueue for Refusing {
    async fn pending(&self) -> PortResult<Vec<ReviewItem>> {
        unavailable("O2")
    }

    async fn decide(
        &self,
        _act: OperatorAct,
        _item_id: &str,
        _decision: ReviewDecision,
    ) -> PortResult<ActionOutcome> {
        unavailable("O2")
    }
}

// TODO(O2): the catalog owner must export the account asset projection.
impl AssetsPort for Refusing {
    async fn list(
        &self,
        _account: AccountId,
        _kind: Option<AssetKind>,
        _campaign_id: Option<&str>,
        _cursor: Option<&str>,
    ) -> PortResult<AssetsView> {
        unavailable("O2")
    }

    async fn set_hold(
        &self,
        _act: OperatorAct,
        _account: AccountId,
        _asset_id: &str,
        _held: bool,
    ) -> PortResult<ActionOutcome> {
        unavailable("O2")
    }
}

// TODO(B): preferences/export adapters need the owning durable account service.
impl AccountExtrasPort for Refusing {
    async fn preferences(&self, _account: AccountId) -> PortResult<EmailPreferences> {
        unavailable("B")
    }

    async fn save_preferences(
        &self,
        _account: AccountId,
        _preferences: EmailPreferences,
        _operation: OperationId,
    ) -> PortResult<()> {
        unavailable("B")
    }

    async fn export_status(&self, _account: AccountId) -> PortResult<(ExportStatus, Option<u64>)> {
        unavailable("B")
    }

    async fn request_export(
        &self,
        _account: AccountId,
        _operation: OperationId,
    ) -> PortResult<(ExportStatus, Option<u64>)> {
        unavailable("B")
    }
}

pub(crate) struct HostedPorts {
    pub(crate) finance: crate::finance::Finance,
    pub(crate) campaigns: crate::campaigns::Campaigns,
    pub(crate) checkout: crate::checkout::Checkout,
    pub(crate) unavailable: Refusing,
}
impl Ports for HostedPorts {
    type Finance = crate::finance::Finance;
    type Checkout = crate::checkout::Checkout;
    type Campaigns = crate::campaigns::Campaigns;
    type Operators = Refusing;
    type Review = Refusing;
    type Assets = Refusing;
    type Extras = Refusing;
    fn finance(&self) -> &Self::Finance {
        &self.finance
    }
    fn checkout(&self) -> &Self::Checkout {
        &self.checkout
    }
    fn campaigns(&self) -> &Self::Campaigns {
        &self.campaigns
    }
    fn operators(&self) -> &Self::Operators {
        &self.unavailable
    }
    fn review(&self) -> &Self::Review {
        &self.unavailable
    }
    fn assets(&self) -> &Self::Assets {
        &self.unavailable
    }
    fn extras(&self) -> &Self::Extras {
        &self.unavailable
    }
}

pub(super) fn unavailable<T>(owner: &'static str) -> PortResult<T> {
    df_observe::record_audit_line(
        "hosted.owner_port_unavailable",
        format!("HOSTED owner_port_unavailable owner={owner}"),
    );
    Err(PortError::Unavailable)
}
