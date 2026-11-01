//! Durable selected usage is the only input to stage settlement.
use super::{database_error, execution_error};
use crate::imagery_publication::StageEconomicUse;
use df_commerce::draft_image::{StageCall, StageRoute};
use df_finance::draft_image::{
    Error, Settlement, StageSettlementCommand, StageSettlementPort, StageUsageEvidence, Usability,
};
use df_finance::service::FinanceError;
use df_finance::{ImageStagePhase, ProviderSettlementPort, ProviderStageSettlement};
use df_imagery::execution::{DispatchId, DispatchSettlement, ExecutionStageId, FinanceTuple};
use df_imagery::runtime::{DecisionValidity, QualifiedDecision, QualifiedNoOutput};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

/// Holds authority/clock context only; the command names the durable raw usage revision.
pub struct CanonicalStageSettlement {
    pub owner: ImageOwner,
    pub now_ms: i64,
}
fn units(values: &[(String, u64)]) -> String {
    Value::Array(
        values
            .iter()
            .map(|(category, quantity)| json!({"category":category,"quantity":quantity}))
            .collect(),
    )
    .to_string()
}
#[derive(Clone, Copy)]
enum EconomicEvidence<'a> {
    Image,
    Decision(&'a QualifiedDecision),
    NoOutput(&'a QualifiedNoOutput),
    Pending,
}
struct UsageMatch<'a> {
    call: &'a StageCall,
    route: &'a StageRoute,
    finance: FinanceTuple,
    account: Option<Vec<u8>>,
    campaign: Option<Vec<u8>>,
    revision: u32,
    evidence: StageUsageEvidence,
    units: &'a [(String, u64)],
    usability: Usability,
    proof: EconomicEvidence<'a>,
}
impl CanonicalStageSettlement {
    async fn verify_usage(&self, tx: &Transaction<'_>, usage: UsageMatch<'_>) -> Result<(), Error> {
        if self.now_ms <= 0 {
            return Err(Error::InvalidRecord);
        }
        let call = usage.call;
        let route = usage.route;
        if let EconomicEvidence::Decision(decision) = usage.proof {
            let scope = decision.scope();
            if scope.owner != self.owner
                || scope.job.as_bytes() != &call.job.0
                || scope.stage.as_bytes() != &call.stage.0
                || scope.dispatch.as_bytes() != &call.dispatch
                || decision.usage_revision() != usage.revision
            {
                return Err(Error::LinkConflict);
            }
        }
        if let EconomicEvidence::NoOutput(proof) = usage.proof {
            let scope = proof.scope();
            if !call.kind.is_standalone()
                || call.planned_publication.is_some()
                || usage.evidence == StageUsageEvidence::Unknown
                || usage.usability != Usability::NoUsableOutput
                || scope.owner != self.owner
                || scope.job.as_bytes() != &call.job.0
                || scope.stage.as_bytes() != &call.stage.0
                || scope.dispatch.as_bytes() != &call.dispatch
                || proof.usage_revision() != usage.revision
            {
                return Err(Error::LinkConflict);
            }
        }
        let pending = matches!(usage.proof, EconomicEvidence::Pending);
        if pending && usage.evidence == StageUsageEvidence::Unknown {
            return Err(Error::InvalidRecord);
        }
        let usable = usage.usability == Usability::Usable;
        // Serialize the latest-revision check with the raw usage journal's append lock.
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
            &[&call.dispatch.as_slice()],
        )
        .await
        .map_err(database_error)?;
        let row=tx.query_opt("SELECT u.usable AS raw_valid,u.raw_output_sha256,octet_length(u.raw_output)::bigint AS raw_bytes,d.dispatch_state FROM imagery.image_dispatch_usage u \
            JOIN imagery.image_finance_admission_binding b ON b.dispatch_id=u.dispatch_id \
            JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=u.dispatch_id \
            WHERE u.dispatch_id=$1 AND u.owner_scope=$2 AND u.job_id=$3 AND u.stage_id=$4 AND u.revision=$5 \
            AND u.provider=$6 AND u.model=$7 AND u.cause=$8 AND u.detail=$9 \
            AND (SELECT jsonb_agg(v ORDER BY (v->>'category') COLLATE \"C\") FROM jsonb_array_elements(u.maxima)v)=$10::text::jsonb \
            AND (SELECT count(*)=count(DISTINCT v->>'category') FROM jsonb_array_elements(u.maxima)v) \
            AND u.evidence=$11 \
            AND (SELECT jsonb_agg(v ORDER BY (v->>'category') COLLATE \"C\") FROM jsonb_array_elements(CASE WHEN u.evidence='unknown' OR jsonb_array_length(u.units)=0 THEN u.maxima ELSE u.units END)v)=$12::text::jsonb \
            AND (SELECT count(*)=count(DISTINCT v->>'category') FROM jsonb_array_elements(u.units)v) \
            AND b.finance_attempt=$13 AND b.operation_id=$14 AND b.effect_id=$15 AND b.account_id IS NOT DISTINCT FROM $16 \
            AND b.stage_call=$17::text::jsonb AND b.route_key=$18 AND b.campaign_id IS NOT DISTINCT FROM $19 AND b.cost_route=$20::text::jsonb \
            AND b.owner_scope=u.owner_scope AND b.job_id=u.job_id AND b.execution_stage_id=u.stage_id \
            AND d.finance_attempt=b.finance_attempt AND d.finance_operation=b.operation_id AND d.finance_effect=b.effect_id \
            AND (($21 AND u.evidence<>'unknown' AND d.dispatch_state='unknown' AND d.settlement='held' \
              AND d.usage_revision IN(0,u.revision) \
              AND EXISTS(SELECT 1 FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) \
                WHERE a.attempt_id=b.finance_attempt AND a.reservation_id=b.reservation_id \
                AND a.account_id IS NOT DISTINCT FROM b.account_id AND a.campaign_id IS NOT DISTINCT FROM b.campaign_id \
                AND a.customer_state='held' AND a.supplier_state='exposure_open' AND r.state='held' \
                AND r.charged_micro_dollars=0 AND r.released_micro_dollars=0 \
                AND a.supplier_exposure_micro_dollars=a.supplier_maximum_micro_dollars \
                AND a.dispatch_state IN('sent','outcome_unknown','failed_unknown')) \
              AND EXISTS(SELECT 1 FROM imagery.image_dispatch_sources consumed WHERE consumed.dispatch_id=d.dispatch_id \
                AND consumed.owner_scope=u.owner_scope AND consumed.job_id=u.job_id AND consumed.stage_id=u.stage_id)) \
              OR (NOT $21 AND (u.evidence='unknown' AND (d.dispatch_state='unknown' OR (d.dispatch_state IN('succeeded','declined') AND d.usage_revision=u.revision)) OR u.evidence<>'unknown' \
                AND d.usage_revision=u.revision AND (u.usable AND d.dispatch_state='succeeded' OR NOT u.usable AND d.dispatch_state IN ('failed','succeeded'))))) \
            AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage later WHERE later.dispatch_id=u.dispatch_id AND later.revision>u.revision)",
            &[&call.dispatch.as_slice(),&owner_scope_bytes(&self.owner),&call.job.0.as_slice(),&call.stage.0.as_slice(),
              &i64::from(usage.revision),&route.provider,&route.model,&route.cause,&route.detail,
              &units(&route.maximum_units),&usage.evidence.label(),&units(usage.units),
              &usage.finance.attempt.as_slice(),&usage.finance.operation.as_slice(),&usage.finance.effect.as_slice(),
              &usage.account,&json!(call).to_string(),&route.key.as_slice(),&usage.campaign,&json!(route).to_string(),&pending])
            .await.map_err(database_error)?;
        let row = row.ok_or(Error::LinkConflict)?;
        if usage.evidence == StageUsageEvidence::Unknown {
            if row.get::<_, String>("dispatch_state") == "declined" {
                self.verify_declined(tx, &usage, &row).await?;
            }
            return Ok(());
        }
        let job = ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::InvalidRecord)?;
        let stage =
            ExecutionStageId::from_bytes(&call.stage.0).map_err(|_| Error::InvalidRecord)?;
        let dispatch = DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::InvalidRecord)?;
        let basis = crate::imagery_execution::read_stage_basis_in(tx, &self.owner, &job, &stage)
            .await
            .map_err(execution_error)?;
        if pending {
            if basis.kind.label() != call.kind.label()
                || basis.input_revision != call.input_revision
                || basis.input_digest != call.input_digest
                || basis.config_revision != call.config_revision
                || basis.config_digest != call.config_digest
                || basis.parent_dispatch.map(|id| *id.as_bytes()) != call.parent_dispatch
            {
                return Err(Error::LinkConflict);
            }
            return Err(Error::EconomicProofPending);
        }
        if let EconomicEvidence::NoOutput(proof) = usage.proof {
            let metadata=tx.query_one("SELECT intent_digest,config_digest FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2",
                &[&job.as_bytes().as_slice(),&owner_scope_bytes(&self.owner)]).await.map_err(database_error)?;
            let intent: [u8; 32] = metadata
                .get::<_, Vec<u8>>("intent_digest")
                .try_into()
                .map_err(|_| Error::InvalidRecord)?;
            let config: [u8; 32] = metadata
                .get::<_, Vec<u8>>("config_digest")
                .try_into()
                .map_err(|_| Error::InvalidRecord)?;
            let raw: Option<Vec<u8>> = row.get("raw_output_sha256");
            if row.get::<_, bool>("raw_valid")
                || row.get::<_, String>("dispatch_state") != "failed"
                || proof.basis() != &basis
                || raw.as_deref() != proof.raw_sha256().map(|value| value.as_slice())
                || !proof.matches_financial_metadata(&self.owner, &job, &intent, &config)
            {
                return Err(Error::LinkConflict);
            }
            return Ok(());
        }
        let economic = match usage.proof {
            EconomicEvidence::Decision(decision) => {
                crate::imagery_publication::decision_economic_use_in(
                    tx,
                    &self.owner,
                    &job,
                    &stage,
                    &dispatch,
                    decision,
                )
                .await
            }
            EconomicEvidence::Image => {
                crate::imagery_publication::stage_economic_use_in(
                    tx,
                    &self.owner,
                    &job,
                    &stage,
                    &dispatch,
                )
                .await
            }
            EconomicEvidence::NoOutput(_) | EconomicEvidence::Pending => {
                return Err(Error::LinkConflict);
            }
        };
        let proof = economic
            .map_err(|_| Error::PortUnavailable)?
            .ok_or(Error::EconomicProofPending)?;
        if proof.usage_revision() != usage.revision {
            return Err(Error::LinkConflict);
        }
        let raw_valid: bool = row.try_get("raw_valid").map_err(|_| Error::InvalidRecord)?;
        let expected = match proof {
            StageEconomicUse::SelectedCandidate { .. } => {
                if !raw_valid {
                    return Err(Error::LinkConflict);
                }
                true
            }
            StageEconomicUse::SupersededCandidate { .. }
            | StageEconomicUse::UndeliveredCandidate { .. }
            | StageEconomicUse::NoOutput { .. } => false,
            StageEconomicUse::Decision { .. } => match usage.proof {
                EconomicEvidence::Decision(decision) => match decision.validity() {
                    DecisionValidity::Useful | DecisionValidity::Uncertain => true,
                    DecisionValidity::Unjudged => false,
                },
                _ => return Err(Error::EconomicProofPending),
            },
        };
        if usable != expected {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
    async fn mirror_usage(
        &self,
        tx: &Transaction<'_>,
        call: &StageCall,
        evidence: StageUsageEvidence,
        usability: Usability,
        replay: bool,
    ) -> Result<(), Error> {
        let settlement = if evidence == StageUsageEvidence::Unknown {
            DispatchSettlement::UnknownOutstanding
        } else if usability == Usability::Usable {
            DispatchSettlement::Settled
        } else {
            DispatchSettlement::UnusableSettled
        };
        let job = ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::InvalidRecord)?;
        let stage =
            ExecutionStageId::from_bytes(&call.stage.0).map_err(|_| Error::InvalidRecord)?;
        let dispatch = DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::InvalidRecord)?;
        if replay {
            let stored = crate::imagery_execution::read_dispatch_in(tx, &self.owner, &dispatch)
                .await
                .map_err(execution_error)?;
            if stored.settlement != settlement {
                return Err(Error::LinkConflict);
            }
        } else {
            crate::imagery_execution::mirror_settlement_in(
                tx,
                &self.owner,
                &job,
                &stage,
                &dispatch,
                settlement,
                self.now_ms,
            )
            .await
            .map_err(execution_error)?;
        }
        Ok(())
    }
}
impl StageSettlementPort for CanonicalStageSettlement {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
    ) -> Result<(), Error> {
        self.verify_account(tx, command, EconomicEvidence::Image)
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
        settlement: &Settlement,
    ) -> Result<(), Error> {
        self.mirror_usage(
            tx,
            command.quote.call(),
            command.evidence,
            command.usability,
            settlement.replay,
        )
        .await
    }
}
fn finance_error(error: Error) -> FinanceError {
    match error {
        Error::Finance(error) => error,
        Error::PortUnavailable => FinanceError::PortUnavailable,
        Error::EconomicProofPending => FinanceError::ImageStageEconomicProofPending,
        _ => FinanceError::ImageStageConflict,
    }
}
impl ProviderSettlementPort for CanonicalStageSettlement {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
    ) -> Result<(), FinanceError> {
        self.verify_provider(tx, command, EconomicEvidence::Image)
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
        phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        self.mirror_usage(
            tx,
            &command.binding.call,
            command.evidence,
            command.usability,
            phase == ImageStagePhase::Replay,
        )
        .await
        .map_err(finance_error)
    }
}

impl CanonicalStageSettlement {
    async fn verify_account(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
        proof: EconomicEvidence<'_>,
    ) -> Result<(), Error> {
        let binding = command.quote.binding();
        if self.owner
            != (ImageOwner::Draft {
                account: binding.scope.account,
            })
        {
            return Err(Error::LinkConflict);
        }
        self.verify_usage(
            tx,
            UsageMatch {
                call: command.quote.call(),
                route: command.quote.route(),
                finance: FinanceTuple::new(binding.attempt, binding.operation, binding.effect)
                    .map_err(|_| Error::InvalidRecord)?,
                account: Some(binding.scope.account.as_bytes().to_vec()),
                campaign: None,
                revision: command.usage_revision,
                evidence: command.evidence,
                units: &command.actual_units,
                usability: command.usability,
                proof,
            },
        )
        .await
    }
}

impl CanonicalStageSettlement {
    async fn verify_provider(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
        proof: EconomicEvidence<'_>,
    ) -> Result<(), FinanceError> {
        let binding = &command.binding;
        if binding.owner_scope.as_bytes() != owner_scope_bytes(&self.owner)
            || !matches!(&self.owner,ImageOwner::Run{campaign,..} if campaign.as_bytes()==binding.campaign.as_bytes())
        {
            return Err(FinanceError::ImageStageConflict);
        }
        self.verify_usage(
            tx,
            UsageMatch {
                call: &binding.call,
                route: &binding.route,
                finance: FinanceTuple::new(
                    *binding.attempt.as_bytes(),
                    *binding.operation.as_bytes(),
                    *binding.effect.as_bytes(),
                )
                .map_err(|_| FinanceError::ImageStageConflict)?,
                account: None,
                campaign: Some(binding.campaign.as_bytes().to_vec()),
                revision: command.usage_revision,
                evidence: command.evidence,
                units: &command.actual_units,
                usability: command.usability,
                proof,
            },
        )
        .await
        .map_err(finance_error)
    }
}

/// Carries a private runtime qualification; the locked reader rechecks its exact immutable lineage.
pub struct CanonicalDecisionSettlement<'a> {
    settlement: &'a CanonicalStageSettlement,
    decision: &'a QualifiedDecision,
}
impl CanonicalStageSettlement {
    pub fn with_decision<'a>(
        &'a self,
        decision: &'a QualifiedDecision,
    ) -> CanonicalDecisionSettlement<'a> {
        CanonicalDecisionSettlement {
            settlement: self,
            decision,
        }
    }
}
impl StageSettlementPort for CanonicalDecisionSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
    ) -> Result<(), Error> {
        self.settlement
            .verify_account(tx, command, EconomicEvidence::Decision(self.decision))
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
        settlement: &Settlement,
    ) -> Result<(), Error> {
        StageSettlementPort::mirror(self.settlement, tx, command, settlement).await
    }
}
impl ProviderSettlementPort for CanonicalDecisionSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
    ) -> Result<(), FinanceError> {
        self.settlement
            .verify_provider(tx, command, EconomicEvidence::Decision(self.decision))
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
        phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        ProviderSettlementPort::mirror(self.settlement, tx, command, phase).await
    }
}

/// Authorizes only definitive failed standalone analytical output; image failures still need NoDelivery.
pub struct CanonicalNoOutputSettlement<'a> {
    settlement: &'a CanonicalStageSettlement,
    proof: &'a QualifiedNoOutput,
}
impl CanonicalStageSettlement {
    pub fn with_no_output<'a>(
        &'a self,
        proof: &'a QualifiedNoOutput,
    ) -> CanonicalNoOutputSettlement<'a> {
        CanonicalNoOutputSettlement {
            settlement: self,
            proof,
        }
    }
}
impl StageSettlementPort for CanonicalNoOutputSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
    ) -> Result<(), Error> {
        self.settlement
            .verify_account(tx, command, EconomicEvidence::NoOutput(self.proof))
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &StageSettlementCommand,
        result: &Settlement,
    ) -> Result<(), Error> {
        StageSettlementPort::mirror(self.settlement, tx, command, result).await
    }
}
impl ProviderSettlementPort for CanonicalNoOutputSettlement<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
    ) -> Result<(), FinanceError> {
        self.settlement
            .verify_provider(tx, command, EconomicEvidence::NoOutput(self.proof))
            .await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
        phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        ProviderSettlementPort::mirror(self.settlement, tx, command, phase).await
    }
}

mod pending;
pub use pending::CanonicalPendingSettlement;

mod decline;
