//! Immutable campaign image admission binding (task IMG-ADMISSION-BINDING-A1).
//!
//! What it owns: the [`ImageStagePort`] half of one unified imagery admission,
//! run inside finance's transaction after `commit_plan` and the fence/media
//! guard. On `New` it admits the catalog row, locks the imagery job, and
//! writes the immutable allocation plus the exact finance receipt replay; on
//! `Replay` it verifies the stored linkage read-only. What it never does: no
//! commit, no second transaction, no provider or network call, no quota
//! reservation on replay, no row repair, no dispatch, and no child retry
//! admission (a preexisting allocation under a new receipt is a conflict).
//!
//! Lock order: finance rows (held by the outer transaction), then the media
//! quota/asset rows via [`admit_generation`](crate::media_catalog::admit_generation),
//! then the imagery job row `FOR UPDATE`, then the allocation/binding inserts.
//! The run/session authority is consulted before any of this module's locks,
//! so holding the job never leads into an inverse finance lock. A withdraw
//! transaction locks the job before its interests and never touches
//! media/finance rows, so this order adds no inverse edge.
//!
//! The constructor takes everything it needs: the validated catalog
//! [`Admission`](df_assets::Admission), the canonical run owner, the expected
//! frozen recipe/config snapshots with the resolved key, the job revision the
//! freeze left behind, and the injected `now_ms`. Nothing reads ambient
//! configuration, the clock, or a provider.
//!
//! Production assembly must supply the real [`RunSessionAuthority`]
//! mapping; tests use an explicit deterministic fake. The constructor checks
//! shapes only and never authenticates anyone.
//!
//! Requires the `df-finance` and `df-commerce` dependencies (root adds them;
//! neither crate depends on `df-persistence`, so this direction is acyclic).
//!
//! The whole admission transaction runs as the `df_imagery_admission`
//! composition role from migration 0039: created NOLOGIN, inheriting the
//! ordinary `df_finance` SELECT and EXECUTE permission, plus the exact media
//! quota/asset and imagery SELECT/lock/insert privileges the statements here
//! need. PostgreSQL requires UPDATE on at least one column for `FOR UPDATE`,
//! so the role holds narrow `UPDATE(updated_at_ms)` on `media.asset` and
//! `imagery.image_job` only: it can touch that timestamp but never state,
//! attempt, owner or snapshot columns (42501). Plain `df_finance` cannot read
//! or write those rows (42501), and the composition role cannot write finance
//! tables directly; finance rows change only through the SECURITY DEFINER
//! `lock_flow`/`commit_event` functions. Deployment adds LOGIN and a password
//! per environment, as with the 0018 runtime roles; an existing role keeps its
//! LOGIN setting.

use std::future::Future;
use std::pin::Pin;

use df_assets::{Admission, Audience, DedupeScope, MediaRequestKey};
use df_commerce::snapshot::ReceiptRow;
use df_finance::{
    ImageStageBinding, ImageStageHookOutcome, ImageStagePort, PublicationAttemptId, StageRefusal,
    service::FinanceError,
};
use df_imagery::{ImageJobId as JournalJobId, ImageOwner, VersionedSnapshot, owner_scope_bytes};
use tokio_postgres::Transaction;

pub(crate) mod rows;

/// Largest frozen snapshot the allocation record keeps, in bytes. The
/// constructor and migration 0039 enforce the same bound.
pub const MAXIMUM_ALLOCATION_SNAPSHOT_BYTES: usize = 1_048_576;

/// Migration 0039, applied by the composition root after media 0010,
/// finance 0020/0022 and imagery 0038. Exposed here so root needs no shared
/// migration-list edit to install it.
pub const IMAGERY_ADMISSION_MIGRATION_VERSION: i32 = 39;

/// The exact SQL of migration 0039 for the composition installer.
pub const IMAGERY_ADMISSION_MIGRATION_SQL: &str =
    include_str!("../migrations/0039_imagery_admission_bindings.sql");

/// Why the run/session check failed. Carries no owner bytes: a scope
/// conflict must never leak another owner id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunSessionError {
    /// The canonical campaign/run does not match the finance
    /// tenant/campaign/session of this admission.
    Refused,
    /// The mapping store was unreachable; nothing was written.
    Unavailable {
        sqlstate: Option<String>,
        detail: String,
    },
}

/// The narrow injected authority that verifies the canonical campaign/run
/// against the finance tenant/campaign/session inside the supplied
/// transaction. No provider or network call; no default allow-all: every
/// implementation must check, and production assembly supplies the real
/// host mapping later.
pub trait RunSessionAuthority: Send + Sync {
    fn verify_canonical_run(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        binding: &ImageStageBinding,
        receipt: &ReceiptRow,
    ) -> impl Future<Output = Result<(), RunSessionError>> + Send;
}

/// Why the port configuration was refused. Nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortConfigError {
    NonCampaignAdmission,
    NonGeneratedAdmission,
    DraftOwnerUnsupported,
    OwnerCampaignMismatch,
    AudienceWithheld,
    AudienceCampaignMismatch,
    RequestKeyMismatch,
    DedupeScopeMismatch,
    ZeroStage,
    EmptySnapshot,
    SnapshotTooLarge,
    BadJobRevision,
    BadTime,
}

/// A campaign admission may only carry an audience that names exactly this
/// campaign, or no campaign at all. A cross-campaign shared/host/members
/// audience and a Withheld audience are incompatible with the campaign that
/// pays for the image. `PublicStatic` and `OwnerPrivate` name no campaign and
/// stay allowed; catalog hardening beyond this port ownership is separate work.
fn check_audience(
    audience: &Audience,
    campaign: &df_types::CampaignId,
) -> Result<(), PortConfigError> {
    match audience {
        Audience::Withheld => Err(PortConfigError::AudienceWithheld),
        Audience::CampaignShared(id) | Audience::Host(id) | Audience::Members(id, _) => {
            if id == campaign {
                Ok(())
            } else {
                Err(PortConfigError::AudienceCampaignMismatch)
            }
        }
        Audience::PublicStatic | Audience::OwnerPrivate(_) => Ok(()),
    }
}

/// Everything [`ImageryAdmissionPort`] is built from. One value per admission
/// shape; a changed job revision or deadline rebuilds it.
#[derive(Clone, Debug)]
pub struct ImageryAdmissionConfig {
    pub admission: Admission,
    pub owner: ImageOwner,
    pub job: JournalJobId,
    pub stage: [u8; 16],
    pub recipe_key: MediaRequestKey,
    pub recipe: VersionedSnapshot,
    pub config: VersionedSnapshot,
    pub expected_job_revision: u64,
    pub absolute_deadline_ms: i64,
    pub now_ms: i64,
}

/// The finance [`ImageStagePort`] backed by real catalog/journal rows in the
/// caller's transaction. Clones share nothing mutable; the struct is data.
pub struct ImageryAdmissionPort<A> {
    authority: A,
    admission: Admission,
    campaign: df_types::CampaignId,
    owner: ImageOwner,
    owner_scope: Vec<u8>,
    job: JournalJobId,
    stage: [u8; 16],
    recipe_key: MediaRequestKey,
    recipe: VersionedSnapshot,
    config: VersionedSnapshot,
    expected_job_revision: u64,
    absolute_deadline_ms: i64,
    now_ms: i64,
}

fn snapshot_valid(snapshot: &VersionedSnapshot) -> Result<(), PortConfigError> {
    if snapshot.bytes().is_empty() {
        return Err(PortConfigError::EmptySnapshot);
    }
    if snapshot.bytes().len() > MAXIMUM_ALLOCATION_SNAPSHOT_BYTES {
        return Err(PortConfigError::SnapshotTooLarge);
    }
    Ok(())
}

impl<A: RunSessionAuthority> ImageryAdmissionPort<A> {
    /// Validates the bounded admission shape and the campaign-only mapping.
    /// A draft owner, a non-campaign admission, or non-generated image bytes
    /// are refused here, before any transaction opens.
    pub fn new(authority: A, config: ImageryAdmissionConfig) -> Result<Self, PortConfigError> {
        use df_assets::{AssetKind, Provenance, QuotaScope};
        use df_types::CampaignId;

        let campaign: CampaignId = config
            .admission
            .campaign
            .ok_or(PortConfigError::NonCampaignAdmission)?;
        if config.admission.kind != AssetKind::Image
            || config.admission.provenance != Provenance::Generated
        {
            return Err(PortConfigError::NonGeneratedAdmission);
        }
        if config.admission.charge_scope != QuotaScope::Campaign(campaign) {
            return Err(PortConfigError::NonCampaignAdmission);
        }
        check_audience(&config.admission.audience, &campaign)?;
        let ImageOwner::Run {
            campaign: owner_campaign,
            ..
        } = &config.owner
        else {
            return Err(PortConfigError::DraftOwnerUnsupported);
        };
        if owner_campaign != &campaign {
            return Err(PortConfigError::OwnerCampaignMismatch);
        }
        if config.admission.request_key != Some(config.recipe_key) {
            return Err(PortConfigError::RequestKeyMismatch);
        }
        if config.admission.dedupe_scope != DedupeScope::campaign(&campaign) {
            return Err(PortConfigError::DedupeScopeMismatch);
        }
        if config.stage == [0; 16] {
            return Err(PortConfigError::ZeroStage);
        }
        snapshot_valid(&config.recipe)?;
        snapshot_valid(&config.config)?;
        if config.expected_job_revision == 0 {
            return Err(PortConfigError::BadJobRevision);
        }
        // Historical replay rebuilds this port after the deadline has passed,
        // so expiry is enforced on the New path (see `check_frozen_job`),
        // never here. Only non-positive times are refused at construction.
        if config.now_ms <= 0 || config.absolute_deadline_ms <= 0 {
            return Err(PortConfigError::BadTime);
        }
        let owner_scope = owner_scope_bytes(&config.owner);
        Ok(Self {
            authority,
            admission: config.admission,
            campaign,
            owner: config.owner,
            owner_scope,
            job: config.job,
            stage: config.stage,
            recipe_key: config.recipe_key,
            recipe: config.recipe,
            config: config.config,
            expected_job_revision: config.expected_job_revision,
            absolute_deadline_ms: config.absolute_deadline_ms,
            now_ms: config.now_ms,
        })
    }

    /// The binding must name this port's owner scope, asset, publication
    /// attempt, job, stage and finance campaign; anything else is a conflict,
    /// never a silent re-admission under a new mapping.
    fn check_binding(&self, binding: &ImageStageBinding) -> Result<(), FinanceError> {
        let same = binding.asset.as_bytes() == self.admission.asset.as_bytes()
            && binding.publication_attempt.as_bytes() == self.admission.attempt.as_bytes()
            && binding.job.as_bytes() == self.job.as_bytes()
            && binding.stage.as_bytes() == &self.stage
            && binding.campaign.as_bytes() == self.campaign.as_bytes()
            && binding.owner_scope.as_bytes() == self.owner_scope.as_slice();
        if same {
            Ok(())
        } else {
            rows::record_conflict("binding_mismatch");
            Err(FinanceError::ImageStageConflict)
        }
    }

    /// Finance scope first: the authority verdict and the exact stored
    /// finance mapping, before any media or job lock of this module.
    async fn check_finance_scope(
        &self,
        tx: &Transaction<'_>,
        binding: &ImageStageBinding,
        receipt: &ReceiptRow,
    ) -> Result<(), FinanceError> {
        if receipt.attempt.as_bytes() != binding.attempt.as_bytes() {
            rows::record_conflict("receipt_attempt_mismatch");
            return Err(FinanceError::ImageStageConflict);
        }
        if let Err(error) = self
            .authority
            .verify_canonical_run(tx, &self.owner, binding, receipt)
            .await
        {
            return Err(match error {
                RunSessionError::Refused => {
                    rows::record_conflict("run_session_refused");
                    FinanceError::ImageStageConflict
                }
                RunSessionError::Unavailable { sqlstate, detail } => {
                    rows::record_unavailable(sqlstate.as_deref());
                    FinanceError::ImageStageUnavailable { sqlstate, detail }
                }
            });
        }
        rows::check_finance_mapping(tx, binding, receipt).await
    }

    /// The new-phase transaction: finance scope, catalog admission, job
    /// freeze comparison under the job lock, then the immutable writes.
    async fn admit_new(
        &self,
        tx: &Transaction<'_>,
        binding: &ImageStageBinding,
        receipt: &ReceiptRow,
    ) -> Result<ImageStageHookOutcome, FinanceError> {
        rows::require_read_committed(tx).await?;
        self.check_binding(binding)?;
        // The constructor already refused incompatible audiences; this stays a
        // typed conflict before any write for a port built another way.
        if check_audience(&self.admission.audience, &self.campaign).is_err() {
            return Err(rows::conflict_of("audience_mismatch"));
        }
        self.check_finance_scope(tx, binding, receipt).await?;
        let publication_attempt = PublicationAttemptId::new(*self.admission.attempt.as_bytes())
            .map_err(|_| rows::conflict_of("publication_attempt_zero"))?;
        rows::check_no_prior_allocation(tx, &publication_attempt).await?;
        rows::admit_catalog(tx, &self.admission, self.now_ms).await?;
        rows::check_catalog_identity(tx, &self.admission).await?;
        rows::check_frozen_job(
            tx,
            rows::FrozenJobCheck {
                owner_scope: binding.owner_scope.as_bytes(),
                job: &self.job,
                recipe_key: &self.recipe_key,
                recipe: &self.recipe,
                config: &self.config,
                expected_revision: self.expected_job_revision,
                absolute_deadline_ms: self.absolute_deadline_ms,
                now_ms: self.now_ms,
            },
        )
        .await?;
        rows::insert_allocation(
            tx,
            rows::PublicationAllocationInsert {
                publication_attempt,
                admission: &self.admission,
                owner_scope: binding.owner_scope.as_bytes(),
                job: &self.job,
                stage: &self.stage,
                recipe_key: &self.recipe_key,
                recipe: &self.recipe,
                config: &self.config,
                now_ms: self.now_ms,
            },
        )
        .await?;
        rows::insert_binding(
            tx,
            binding,
            receipt,
            binding.owner_scope.as_bytes(),
            self.now_ms,
        )
        .await?;
        Ok(ImageStageHookOutcome::Accepted)
    }

    /// The replay phase: read-only verification of the identical stored
    /// linkage. Never admits, reserves, repairs, dispatches or sends.
    async fn verify_stored(
        &self,
        tx: &Transaction<'_>,
        binding: &ImageStageBinding,
        receipt: &ReceiptRow,
    ) -> Result<ImageStageHookOutcome, FinanceError> {
        rows::require_read_committed(tx).await?;
        self.check_binding(binding)?;
        rows::verify_stored_linkage(
            tx,
            binding,
            receipt,
            &self.recipe_key,
            &self.recipe,
            &self.config,
        )
        .await
    }
}

impl<A: RunSessionAuthority> ImageStagePort for ImageryAdmissionPort<A> {
    fn admit_image_link<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        binding: &'a ImageStageBinding,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        Box::pin(async move { self.admit_new(transaction, binding, receipt).await })
    }

    fn verify_image_link<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        binding: &'a ImageStageBinding,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        Box::pin(async move { self.verify_stored(transaction, binding, receipt).await })
    }
}

/// Maps a catalog refusal to the finance stage refusal without exposing
/// underlying foreign ids. `upload_limit` and `owner_unsupported` are fixed
/// code-like texts for outcomes unreachable from a validated campaign
/// generated image; they never panic.
fn map_not_admitted(not_admitted: &df_assets::NotAdmitted) -> StageRefusal {
    use df_assets::NotAdmitted;
    match not_admitted {
        NotAdmitted::AudienceEmpty => StageRefusal::AudienceEmpty,
        NotAdmitted::InFlight { .. } => StageRefusal::InFlight,
        NotAdmitted::Ready { .. } => StageRefusal::Ready,
        NotAdmitted::QuotaFull(_) => StageRefusal::QuotaFull,
        NotAdmitted::UploadLimit => StageRefusal::Other {
            reason: "upload_limit".to_owned(),
        },
        NotAdmitted::OwnerUnsupported => StageRefusal::Other {
            reason: "owner_unsupported".to_owned(),
        },
    }
}

/// Maps a catalog failure to the finance outcome: decode and scope
/// invariants are safe conflicts/refusals, never default success; a
/// pre-commit failure is unavailable; commit uncertainty propagates without
/// repair. There is no port-local commit, so no commit-unknown is made here.
fn map_catalog_error(error: &df_assets::CatalogError) -> FinanceError {
    use df_assets::CatalogError;
    match error {
        CatalogError::InvalidRecord => {
            rows::record_conflict("catalog_invalid_record");
            FinanceError::ImageStageConflict
        }
        CatalogError::ScopeMoved => {
            rows::record_unavailable(Some("40001"));
            FinanceError::ImageStageUnavailable {
                sqlstate: Some("40001".to_owned()),
                detail: "imagery catalog scope moved".to_owned(),
            }
        }
        CatalogError::Unavailable => {
            rows::record_unavailable(None);
            FinanceError::ImageStageUnavailable {
                sqlstate: None,
                detail: "imagery catalog unavailable".to_owned(),
            }
        }
        CatalogError::Unknown => FinanceError::ImageStageUncertain {
            sqlstate: None,
            detail: "imagery catalog uncertain".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{map_catalog_error, map_not_admitted};
    use df_assets::{CatalogError, NotAdmitted, QuotaRefusal, QuotaScope};
    use df_finance::StageRefusal;
    use df_finance::service::FinanceError;
    use df_types::CampaignId;

    #[test]
    fn every_catalog_refusal_maps_without_foreign_ids() {
        use df_assets::AttemptId as CatalogAttempt;
        assert_eq!(
            map_not_admitted(&NotAdmitted::AudienceEmpty),
            StageRefusal::AudienceEmpty
        );
        assert_eq!(
            map_not_admitted(&NotAdmitted::InFlight {
                attempt: CatalogAttempt::from_bytes(&[1; 16]).expect("attempt"),
            }),
            StageRefusal::InFlight
        );
        assert_eq!(
            map_not_admitted(&NotAdmitted::Ready {
                asset: df_assets::AssetId::from_bytes(&[2; 16]).expect("asset"),
            }),
            StageRefusal::Ready
        );
        assert_eq!(
            map_not_admitted(&NotAdmitted::QuotaFull(QuotaRefusal {
                scope: QuotaScope::Campaign(CampaignId::from_bytes(&[3; 16]).expect("campaign")),
                needed_bytes: 10,
                available_bytes: 1,
            })),
            StageRefusal::QuotaFull
        );
        assert_eq!(
            map_not_admitted(&NotAdmitted::UploadLimit),
            StageRefusal::Other {
                reason: "upload_limit".to_owned(),
            }
        );
        assert_eq!(
            map_not_admitted(&NotAdmitted::OwnerUnsupported),
            StageRefusal::Other {
                reason: "owner_unsupported".to_owned(),
            }
        );
    }

    #[test]
    fn every_catalog_failure_maps_to_a_safe_outcome() {
        assert!(matches!(
            map_catalog_error(&CatalogError::InvalidRecord),
            FinanceError::ImageStageConflict
        ));
        assert!(matches!(
            map_catalog_error(&CatalogError::ScopeMoved),
            FinanceError::ImageStageUnavailable { .. }
        ));
        assert!(matches!(
            map_catalog_error(&CatalogError::Unavailable),
            FinanceError::ImageStageUnavailable { .. }
        ));
        assert!(matches!(
            map_catalog_error(&CatalogError::Unknown),
            FinanceError::ImageStageUncertain { .. }
        ));
    }
}
