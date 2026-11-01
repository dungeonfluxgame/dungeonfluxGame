//! The latest exact raw usage revision drives the existing finance settlement policies.
use super::context::route_matches;
use super::records::{self, AdmittedFunding};
use super::{FundedImageAdmission, RuntimeImageAuthority, draft_error, finance_error};
use crate::imagery_draft_admission::CanonicalStageSettlement;
use df_finance::draft_image::{
    StageSettlementCommand, StageSettlementPort, StageUsageEvidence, Usability,
};
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_finance::service::FinanceService;
use df_finance::{ProviderSettlementPort, ProviderStageSettlement};
use df_imagery::ImageJob;
use df_imagery::cost::UsageEvidence;
use df_imagery::execution::{DispatchId, DispatchMode};
use df_imagery::runtime::{
    DispatchDecisionSettlement, DispatchNoOutputSettlement, ImageDispatchAdmission,
    ImageUsageJournal, QualifiedDecision, QualifiedNoOutput, RuntimeError, RuntimeUsage,
};

/// Canonical finance input selected from retained billing evidence and semantic usability.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct SelectedUsage {
    pub evidence: StageUsageEvidence,
    pub usability: Usability,
    pub units: Vec<(String, u64)>,
}

pub(super) fn selected(
    usage: &RuntimeUsage,
    usable: Option<bool>,
) -> Result<SelectedUsage, RuntimeError> {
    let evidence = match (usage.evidence, usable) {
        (UsageEvidence::Unknown, _) => StageUsageEvidence::Unknown,
        (UsageEvidence::Reported, Some(_)) => StageUsageEvidence::Reported,
        (UsageEvidence::Estimated, Some(_)) => StageUsageEvidence::Estimated,
        _ => return Err(RuntimeError::InvalidRecord),
    };
    // This claim is checked against locked Ready/NoDelivery lineage by the canonical money port.
    // Raw technical validity remains unchanged and is independently checked there for decisions.
    let usability = if usable.unwrap_or(usage.usable) {
        Usability::Usable
    } else {
        Usability::NoUsableOutput
    };
    let canonical = |values: &[df_imagery::cost::CostUnits]| {
        let mut units = values
            .iter()
            .map(|unit| (unit.category.clone(), unit.quantity))
            .collect::<Vec<_>>();
        units.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        if units.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(units)
    };
    let observed = canonical(&usage.units)?;
    let maxima = canonical(&usage.route.maxima)?;
    if maxima.is_empty() || maxima.iter().any(|(_, maximum)| *maximum == 0) {
        return Err(RuntimeError::InvalidRecord);
    }
    // Existing CostUsage semantics use pinned maxima for unknown or absent observations.
    // Raw empty units and their evidence remain unchanged in the durable usage journal.
    let units = if evidence == StageUsageEvidence::Unknown || observed.is_empty() {
        maxima
    } else {
        observed
    };
    Ok(SelectedUsage {
        evidence,
        usability,
        units,
    })
}

/// Only the canonical finance port can authorize a claim against its locked terminal evidence.
pub(super) enum SettlementProof<'a> {
    Image(Option<bool>),
    Pending,
    Decision {
        usage: &'a RuntimeUsage,
        qualified: &'a QualifiedDecision,
    },
    NoOutput {
        usage: &'a RuntimeUsage,
        qualified: &'a QualifiedNoOutput,
    },
}
impl SettlementProof<'_> {
    fn claimed_use(&self) -> Option<bool> {
        match self {
            Self::Image(usable) => *usable,
            Self::Pending => None,
            Self::Decision { qualified, .. } => Some(qualified.is_useful()),
            Self::NoOutput { .. } => Some(false),
        }
    }
}
async fn settle<
    F: FencePort,
    M: MediaPort,
    S: MemberPort,
    P: StageSettlementPort + ProviderSettlementPort,
>(
    finance: &mut FinanceService<F, M, S>,
    stored: AdmittedFunding,
    usage_revision: u32,
    selected: SelectedUsage,
    port: &P,
) -> Result<(), RuntimeError> {
    let SelectedUsage {
        evidence,
        usability,
        units: actual_units,
    } = selected;
    match stored {
        AdmittedFunding::Draft { account, quote } => {
            finance
                .settle_draft_stage(
                    account,
                    &StageSettlementCommand {
                        quote: *quote,
                        usage_revision,
                        evidence,
                        actual_units,
                        usability,
                    },
                    port,
                )
                .await
                .map_err(draft_error)?;
        }
        AdmittedFunding::Campaign(binding) => {
            finance
                .settle_provider_stage(
                    &ProviderStageSettlement {
                        binding: *binding,
                        usage_revision,
                        evidence,
                        actual_units,
                        usability,
                    },
                    port,
                )
                .await
                .map_err(finance_error)?;
        }
    }
    Ok(())
}
impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    FundedImageAdmission<F, M, S, A, L>
{
    pub(super) async fn settle_no_output(
        &self,
        input: &DispatchNoOutputSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        if input.mode == DispatchMode::Local {
            return self.ports.local.outcome_no_output(input).await;
        }
        if input.usage.evidence == UsageEvidence::Unknown
            || input.usage.usable
            || !input.qualified.basis().kind.is_standalone()
        {
            return Err(RuntimeError::InvalidRecord);
        }
        self.settle_stage(
            input.job,
            input.usage.dispatch,
            input.mode,
            SettlementProof::NoOutput {
                usage: input.usage,
                qualified: input.qualified,
            },
        )
        .await
    }
    pub(super) async fn settle_decision(
        &self,
        input: &DispatchDecisionSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        if input.mode == DispatchMode::Local {
            return self.ports.local.outcome_decision(input).await;
        }
        self.settle_stage(
            input.job,
            input.usage.dispatch,
            input.mode,
            SettlementProof::Decision {
                usage: input.usage,
                qualified: input.qualified,
            },
        )
        .await
    }
    pub(super) async fn settle_stage(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        proof: SettlementProof<'_>,
    ) -> Result<(), RuntimeError> {
        if mode == DispatchMode::Local {
            let SettlementProof::Image(usable) = proof else {
                return Err(RuntimeError::InvalidRecord);
            };
            return self.ports.local.outcome(job, dispatch, mode, usable).await;
        }
        let expected = mode.finance().ok_or(RuntimeError::InvalidRecord)?;
        let raw = self
            .ports
            .usage
            .latest_usage(&job.owner, dispatch)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let proof = match proof {
            SettlementProof::Image(None) if raw.evidence != UsageEvidence::Unknown => {
                SettlementProof::Pending
            }
            proof => proof,
        };
        if let SettlementProof::Decision { usage, qualified } = &proof {
            let scope = qualified.scope();
            if *usage != &raw
                || scope.owner != job.owner
                || scope.job != job.job
                || scope.stage != raw.stage
                || scope.dispatch != dispatch
                || u64::from(qualified.usage_revision()) != raw.revision.0
            {
                return Err(RuntimeError::InvalidRecord);
            }
        }
        if let SettlementProof::NoOutput { usage, qualified } = &proof {
            let scope = qualified.scope();
            if *usage != &raw
                || scope.owner != job.owner
                || scope.job != job.job
                || scope.stage != raw.stage
                || scope.dispatch != dispatch
                || u64::from(qualified.usage_revision()) != raw.revision.0
                || raw.evidence == UsageEvidence::Unknown
                || raw.usable
            {
                return Err(RuntimeError::InvalidRecord);
            }
        }
        let mut finance = self.ports.finance.lock().await;
        let stored = records::read_admitted_funding(&finance, job, dispatch).await?;
        if stored.finance()? != expected
            || raw.owner != job.owner
            || raw.job != job.job
            || raw.dispatch != dispatch
            || raw.stage.as_bytes() != &stored.call().stage.0
            || !route_matches(stored.route(), &raw.route)
        {
            return Err(RuntimeError::InvalidRecord);
        }
        if matches!(proof, SettlementProof::NoOutput { .. })
            && (!stored.call().kind.is_standalone() || stored.call().planned_publication.is_some())
        {
            return Err(RuntimeError::InvalidRecord);
        }
        // Pending carries the retained technical bit only; its canonical port returns before any economic posting.
        let claimed_use = if matches!(proof, SettlementProof::Pending) {
            Some(raw.usable)
        } else {
            proof.claimed_use()
        };
        let selected = selected(&raw, claimed_use)?;
        let usage_revision =
            u32::try_from(raw.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
        if usage_revision == 0 {
            return Err(RuntimeError::InvalidRecord);
        }
        let port = CanonicalStageSettlement {
            owner: job.owner.clone(),
            now_ms: finance.now_ms(),
        };
        match proof {
            SettlementProof::Pending => {
                settle(
                    &mut finance,
                    stored,
                    usage_revision,
                    selected,
                    &port.with_pending(),
                )
                .await
            }
            SettlementProof::Image(_) => {
                settle(&mut finance, stored, usage_revision, selected, &port).await
            }
            SettlementProof::Decision { qualified, .. } => {
                settle(
                    &mut finance,
                    stored,
                    usage_revision,
                    selected,
                    &port.with_decision(qualified),
                )
                .await
            }
            SettlementProof::NoOutput { qualified, .. } => {
                settle(
                    &mut finance,
                    stored,
                    usage_revision,
                    selected,
                    &port.with_no_output(qualified),
                )
                .await
            }
        }
    }
}
