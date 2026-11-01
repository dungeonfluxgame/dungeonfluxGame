//! Canonical paid stage binding for both account and campaign finance transactions.

use super::Config;
use df_assets::{AdmitOutcome, StorageOwner};
use df_commerce::draft_image::{PaidStageKind, StageCall};
use df_commerce::snapshot::ReceiptRow;
use df_finance::draft_image::{Error, StageAdmissionPort, StageReceipt};
use df_finance::service::FinanceError;
use df_finance::{ImageStageHookOutcome, ImageStagePhase, ProviderStageBinding, ProviderStagePort};
use df_imagery::execution::{DispatchId, DispatchMode, FinanceTuple, PrepareCommand, StageClaim};
use df_imagery::{ImageOwner, owner_scope_bytes};
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::Transaction;

mod linkage;
mod sending;
mod settlement;
pub use sending::CanonicalStageSend;
pub use settlement::{
    CanonicalDecisionSettlement, CanonicalNoOutputSettlement, CanonicalPendingSettlement,
    CanonicalStageSettlement,
};

/// The host verifies canonical source ownership and campaign/session funding authority.
pub trait StageAuthority: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        payer: StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        replay: bool,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// New records the exact consumed-source snapshot after prepare; replay only compares it.
    fn record_prepared_sources(
        &self,
        tx: &Transaction<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        replay: bool,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}
/// Account identity or the real campaign finance tuple; never an inferred synthetic account.
#[derive(Clone, Copy)]
pub enum StagePayer<'a> {
    Account(df_types::AccountId),
    Campaign(&'a ProviderStageBinding),
}
/// A decision has no catalog rows; later paid children attach the existing original allocation.
pub enum StageAllocation {
    Decision,
    Output(Box<Config>),
}
/// The claim is transient NEW authority; it never changes the stored paid-call fingerprint.
pub struct StageConfig {
    pub claim: StageClaim,
    pub allocation: StageAllocation,
    pub now_ms: i64,
}
pub struct CanonicalStagePort<A> {
    authority: A,
    config: StageConfig,
}
impl<A: StageAuthority> CanonicalStagePort<A> {
    pub fn new(authority: A, config: StageConfig) -> Result<Self, Error> {
        if config.now_ms <= 0 {
            return Err(Error::InvalidRecord);
        }
        if let StageAllocation::Output(output) = &config.allocation {
            let stored =
                df_assets::admission_owner(&output.admission).map_err(|_| Error::CatalogRefused)?;
            let valid = match (&config.claim.scope.owner, stored) {
                (ImageOwner::Draft { account }, Some(StorageOwner::AccountDraft(stored))) => {
                    *account == stored
                }
                (ImageOwner::Run { campaign, .. }, Some(StorageOwner::Campaign(stored))) => {
                    *campaign == stored
                }
                _ => false,
            };
            if !valid
                || output.admission.request_key != Some(output.recipe_key)
                || output.recipe.bytes().is_empty()
                || output.config.bytes().is_empty()
                || output.recipe.bytes().len()
                    > super::super::imagery_admission::MAXIMUM_ALLOCATION_SNAPSHOT_BYTES
                || output.config.bytes().len()
                    > super::super::imagery_admission::MAXIMUM_ALLOCATION_SNAPSHOT_BYTES
            {
                return Err(Error::CatalogRefused);
            }
        }
        Ok(Self { authority, config })
    }

    async fn bind_stage(&self, tx: &Transaction<'_>, link: linkage::Link<'_>) -> Result<(), Error> {
        let scope = &self.config.claim.scope;
        let call = link.call;
        if scope.job.as_bytes() != &call.job.0
            || scope.stage.as_bytes() != &call.stage.0
            || link.owner != &scope.owner
        {
            return Err(admission_gate_error("scope", Error::LinkConflict));
        }
        self.authority
            .verify(tx, link.payer, link.owner, call, link.replay)
            .await
            .map_err(|error| admission_gate_error("authority", error))?;
        linkage::check_finance(tx, &link)
            .await
            .map_err(|error| admission_gate_error("finance_tuple", error))?;
        if link.replay {
            linkage::verify(tx, &link).await?;
            return self
                .authority
                .record_prepared_sources(tx, link.owner, call, true)
                .await;
        }
        // Catalog precedes job/execution locks, matching the existing admission lock order.
        let allocation = self
            .allocate(tx, call, link.at_ms)
            .await
            .map_err(|error| admission_gate_error("allocation", error))?;
        let basis = crate::imagery_execution::read_stage_basis_in(
            tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
        )
        .await
        .map_err(execution_error)?;
        if basis.kind.label() != call.kind.label()
            || basis.input_revision != call.input_revision
            || basis.input_digest != call.input_digest
            || basis.config_revision != call.config_revision
            || basis.config_digest != call.config_digest
            || basis.parent_dispatch.map(|id| *id.as_bytes()) != call.parent_dispatch
        {
            return Err(admission_gate_error("basis", Error::LinkConflict));
        }
        let stage =
            crate::imagery_execution::read_stage_in(tx, &scope.owner, &scope.job, &scope.stage)
                .await
                .map_err(execution_error)?;
        let planned =
            call.planned_publication
                .map(|p| df_imagery::execution::PublicationIdentity {
                    asset: p.asset.0,
                    attempt: p.publication.0,
                });
        if stage.planned_publication != planned || stage.local_accounting {
            return Err(admission_gate_error("stage_identity", Error::LinkConflict));
        }
        let dispatch = DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::InvalidRecord)?;
        let command = PrepareCommand::new(
            self.config.claim.clone(),
            dispatch,
            DispatchMode::Hosted(link.finance),
            link.at_ms,
        )
        .map_err(|_| Error::InvalidRecord)?;
        crate::imagery_execution::prepare_dispatch_in(tx, &command)
            .await
            .map_err(execution_error)
            .map_err(|error| admission_gate_error("prepare", error))?;
        self.authority
            .record_prepared_sources(tx, link.owner, call, false)
            .await
            .map_err(|error| admission_gate_error("sources", error))?;
        linkage::insert(tx, &link, allocation.as_ref())
            .await
            .map_err(|error| admission_gate_error("linkage", error))?;
        df_observe::record_audit_line(
            "IMAGERY",
            format!(
                "IMAGERY stage_admission status=link_staged kind={} dispatch={}",
                call.kind.label(),
                df_commerce::finance_ids::hex(&call.dispatch)
            ),
        );
        Ok(())
    }

    async fn allocate(
        &self,
        tx: &Transaction<'_>,
        call: &StageCall,
        at_ms: i64,
    ) -> Result<Option<Vec<u8>>, Error> {
        let scope = owner_scope_bytes(&self.config.claim.scope.owner);
        match &self.config.allocation {
            StageAllocation::Decision => {
                if call.kind != PaidStageKind::Classification && !call.kind.is_standalone() {
                    return Err(Error::LinkConflict);
                }
                let asset = call.planned_publication.map(|p| p.asset.0.to_vec());
                let publication = call.planned_publication.map(|p| p.publication.0.to_vec());
                let row=tx.query_opt("SELECT 1 FROM imagery.image_job j WHERE j.job_id=$1 AND j.owner_scope=$2 \
                    AND j.state='planning' AND j.planned_publication_asset IS NOT DISTINCT FROM $3 AND j.planned_publication_attempt IS NOT DISTINCT FROM $4 \
                    AND j.absolute_deadline_ms>$5 AND EXISTS(SELECT 1 FROM imagery.image_interest i \
                    WHERE i.job_id=j.job_id AND i.state='attached') FOR UPDATE",
                    &[&call.job.0.as_slice(),&scope,&asset,&publication,&at_ms])
                    .await.map_err(database_error)?;
                if row.is_none() {
                    return Err(Error::LinkConflict);
                }
                Ok(None)
            }
            StageAllocation::Output(output) => {
                let publication = call.planned_publication.ok_or(Error::LinkConflict)?;
                if call.kind == PaidStageKind::Classification
                    || call.kind.is_standalone()
                    || publication.asset.0 != *output.admission.asset.as_bytes()
                    || publication.publication.0 != *output.admission.attempt.as_bytes()
                {
                    return Err(Error::LinkConflict);
                }
                if let Some(anchor) = linkage::allocation(tx, output, &scope, call).await? {
                    return Ok(Some(anchor));
                }
                if !matches!(
                    call.kind,
                    PaidStageKind::Generation | PaidStageKind::Edit | PaidStageKind::Extraction
                ) || at_ms >= output.absolute_deadline_ms
                {
                    return Err(Error::CatalogRefused);
                }
                let outcome = crate::media_catalog::admit_generation(tx, &output.admission, at_ms)
                    .await
                    .map_err(|_| Error::PortUnavailable)?;
                if outcome != AdmitOutcome::Admitted {
                    return Err(Error::CatalogRefused);
                }
                linkage::first_allocation(tx, output, &scope, call, at_ms).await?;
                Ok(Some(call.stage.0.to_vec()))
            }
        }
    }
}

impl<A: StageAuthority> StageAdmissionPort for CanonicalStagePort<A> {
    async fn bind(&self, tx: &Transaction<'_>, receipt: &StageReceipt) -> Result<(), Error> {
        let binding = receipt.quote.binding();
        let owner = ImageOwner::Draft {
            account: binding.scope.account,
        };
        let finance = FinanceTuple::new(binding.attempt, binding.operation, binding.effect)
            .map_err(|_| Error::InvalidRecord)?;
        let link = linkage::Link {
            owner: &owner,
            payer: StagePayer::Account(binding.scope.account),
            call: receipt.quote.call(),
            route: receipt.quote.route(),
            route_key: receipt.quote.route().key,
            finance,
            reservation: *receipt.money.reservation.as_bytes(),
            fingerprint: receipt.money.fingerprint,
            replay: receipt.money.replay,
            at_ms: receipt.money.admitted_at_ms,
        };
        self.bind_stage(tx, link).await
    }
}
impl<A: StageAuthority> ProviderStagePort for CanonicalStagePort<A> {
    fn bind<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        binding: &'a ProviderStageBinding,
        phase: ImageStagePhase,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        Box::pin(async move {
            let finance = FinanceTuple::new(
                *binding.attempt.as_bytes(),
                *binding.operation.as_bytes(),
                *binding.effect.as_bytes(),
            )
            .map_err(|_| FinanceError::ImageStageConflict)?;
            let owner = &self.config.claim.scope.owner;
            if binding.owner_scope.as_bytes() != owner_scope_bytes(owner) {
                return Err(FinanceError::ImageStageConflict);
            }
            let row = tx
                .query_one(
                    "SELECT admitted_at_ms FROM finance.provider_attempt WHERE attempt_id=$1",
                    &[&binding.attempt.as_bytes().as_slice()],
                )
                .await
                .map_err(FinanceError::from)?;
            let link = linkage::Link {
                owner,
                payer: StagePayer::Campaign(binding),
                call: &binding.call,
                route: &binding.route,
                route_key: binding.route_key,
                finance,
                reservation: *receipt.reservation.as_bytes(),
                fingerprint: receipt.fingerprint,
                replay: phase == ImageStagePhase::Replay,
                at_ms: row.get(0),
            };
            self.bind_stage(tx, link)
                .await
                .map_err(|error| match error {
                    Error::Finance(error) => error,
                    Error::PortUnavailable => FinanceError::PortUnavailable,
                    _ => FinanceError::ImageStageConflict,
                })?;
            Ok(if phase == ImageStagePhase::Replay {
                ImageStageHookOutcome::Existing
            } else {
                ImageStageHookOutcome::Accepted
            })
        })
    }
}
fn database_error(error: tokio_postgres::Error) -> Error {
    Error::Finance(error.into())
}
fn execution_error(error: df_imagery::execution::ExecutionError) -> Error {
    match error {
        df_imagery::execution::ExecutionError::Unavailable
        | df_imagery::execution::ExecutionError::Unknown => Error::PortUnavailable,
        _ => Error::LinkConflict,
    }
}

fn admission_gate_error(gate: &'static str, error: Error) -> Error {
    #[cfg(test)]
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY test_stage_admission_refusal gate={gate}"),
    );
    #[cfg(not(test))]
    let _ = gate;
    error
}
