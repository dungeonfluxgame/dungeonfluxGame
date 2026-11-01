//! Known usage on an execution-Unknown dispatch is checked without any posting or mirror.
use super::*;

pub struct CanonicalPendingSettlement<'a> {
    settlement: &'a CanonicalStageSettlement,
}
impl CanonicalStageSettlement {
    pub fn with_pending(&self) -> CanonicalPendingSettlement<'_> {
        CanonicalPendingSettlement { settlement: self }
    }
}
impl StageSettlementPort for CanonicalPendingSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
    ) -> Result<(), Error> {
        self.settlement
            .verify_account(tx, command, EconomicEvidence::Pending)
            .await
    }
    async fn mirror(
        &self,
        _tx: &Transaction<'_>,
        _command: &StageSettlementCommand,
        _settlement: &Settlement,
    ) -> Result<(), Error> {
        Err(Error::EconomicProofPending)
    }
}
impl ProviderSettlementPort for CanonicalPendingSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
    ) -> Result<(), FinanceError> {
        self.settlement
            .verify_provider(tx, command, EconomicEvidence::Pending)
            .await
    }
    async fn mirror(
        &self,
        _tx: &Transaction<'_>,
        _command: &ProviderStageSettlement,
        _phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        Err(FinanceError::ImageStageEconomicProofPending)
    }
}
