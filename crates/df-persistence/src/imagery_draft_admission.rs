//! Private account-draft catalog admission under the existing finance transaction.
//! Uses the same immutable publication allocation as campaign imagery.

use std::future::Future;

use df_assets::{Admission, AdmitOutcome, MediaRequestKey, StorageOwner};
use df_commerce::draft_image::Binding;
use df_finance::draft_image::{AdmissionPort, Error, Receipt};
use df_imagery::{ImageOwner, VersionedSnapshot, owner_scope_bytes};
use tokio_postgres::Transaction;

use crate::imagery_admission::MAXIMUM_ALLOCATION_SNAPSHOT_BYTES;

mod rows;
mod stages;
pub use stages::{
    CanonicalDecisionSettlement, CanonicalNoOutputSettlement, CanonicalPendingSettlement,
    CanonicalStagePort, CanonicalStageSend, CanonicalStageSettlement, StageAllocation,
    StageAuthority, StageConfig, StagePayer,
};

pub const MIGRATION_VERSION: i32 = 43;
pub const MIGRATION_SQL: &str = include_str!("../migrations/0043_imagery_draft_funding.sql");

/// The host checks its canonical draft/source revision and account authority. A
/// historical replay checks present access to the stored job, never mutates it.
pub trait DraftAuthority: Send + Sync {
    fn verify(
        &self,
        transaction: &Transaction<'_>,
        binding: &Binding,
        replay: bool,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

pub struct Config {
    pub admission: Admission,
    pub recipe_key: MediaRequestKey,
    pub recipe: VersionedSnapshot,
    pub config: VersionedSnapshot,
    pub expected_job_revision: u64,
    pub absolute_deadline_ms: i64,
    pub now_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidDraftOwner,
    RequestKeyMismatch,
    InvalidSnapshot,
    InvalidRevision,
    InvalidTime,
}

pub struct DraftAdmissionPort<A> {
    authority: A,
    config: Config,
    account: df_types::AccountId,
    owner_scope: Vec<u8>,
}

impl<A: DraftAuthority> DraftAdmissionPort<A> {
    pub fn new(authority: A, config: Config) -> Result<Self, ConfigError> {
        let owner = df_assets::admission_owner(&config.admission)
            .map_err(|_| ConfigError::InvalidDraftOwner)?;
        let Some(StorageOwner::AccountDraft(account)) = owner else {
            return Err(ConfigError::InvalidDraftOwner);
        };
        if config.admission.request_key != Some(config.recipe_key) {
            return Err(ConfigError::RequestKeyMismatch);
        }
        if config.recipe.bytes().is_empty()
            || config.config.bytes().is_empty()
            || config.recipe.bytes().len() > MAXIMUM_ALLOCATION_SNAPSHOT_BYTES
            || config.config.bytes().len() > MAXIMUM_ALLOCATION_SNAPSHOT_BYTES
        {
            return Err(ConfigError::InvalidSnapshot);
        }
        if config.expected_job_revision == 0 {
            return Err(ConfigError::InvalidRevision);
        }
        if config.now_ms <= 0 || config.absolute_deadline_ms <= 0 {
            return Err(ConfigError::InvalidTime);
        }
        Ok(Self {
            authority,
            config,
            account,
            owner_scope: owner_scope_bytes(&ImageOwner::Draft { account }),
        })
    }

    fn check_binding(&self, binding: &Binding, receipt: &Receipt) -> Result<(), Error> {
        if binding.scope.account != self.account
            || binding.asset.0 != *self.config.admission.asset.as_bytes()
            || binding.publication.0 != *self.config.admission.attempt.as_bytes()
            || binding.attempt != *receipt.attempt.as_bytes()
            || binding.scope.config_digest
                != df_commerce::digest::sha256(self.config.config.bytes())
        {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
}

impl<A: DraftAuthority> AdmissionPort for DraftAdmissionPort<A> {
    async fn bind(
        &self,
        tx: &Transaction<'_>,
        binding: &Binding,
        receipt: &Receipt,
    ) -> Result<(), Error> {
        self.check_binding(binding, receipt)?;
        self.authority.verify(tx, binding, receipt.replay).await?;
        rows::check_finance(tx, self.account, binding, receipt).await?;
        if receipt.replay {
            return rows::verify_link(tx, &self.config, &self.owner_scope, binding, receipt).await;
        }
        if receipt.admitted_at_ms >= self.config.absolute_deadline_ms {
            return Err(Error::CatalogRefused);
        }
        let outcome = crate::media_catalog::admit_generation(
            tx,
            &self.config.admission,
            receipt.admitted_at_ms,
        )
        .await
        .map_err(|_| Error::PortUnavailable)?;
        if outcome != AdmitOutcome::Admitted {
            return Err(Error::CatalogRefused);
        }
        rows::check_catalog(tx, &self.config).await?;
        rows::check_job(
            tx,
            &self.config,
            &self.owner_scope,
            binding,
            receipt.admitted_at_ms,
        )
        .await?;
        rows::insert(tx, &self.config, &self.owner_scope, binding, receipt).await?;
        df_observe::record_audit_line(
            "IMAGERY",
            format!(
                "IMAGERY draft_admission status=link_staged attempt={} job={}",
                receipt.attempt.hex(),
                df_commerce::finance_ids::hex(&binding.scope.job.0)
            ),
        );
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;
