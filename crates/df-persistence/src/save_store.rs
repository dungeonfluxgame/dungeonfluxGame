//! Hosted save-point store (plan 13.6-13.7, SAVE-01..03; decisions 24, 31).
//!
//! A save point holds the checkpoint codec output with its codec/schema/content versions, its
//! campaign, branch and parent save, and forward-only links (by id, no cross-owner foreign key)
//! to finance admission receipts and paid generation attempts. Branch/restore never writes a
//! finance or media row, so money and paid generations stay forward-only. Every mutating
//! operation carries an operation id; its result commits with the state in one transaction and
//! a retry returns the stored result (commit-before-reply).

use std::collections::BTreeMap;

use df_types::{CampaignId, MemberId, OperationId, TenantId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SavePointId([u8; 16]);
impl SavePointId {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        let value: [u8; 16] = bytes.try_into().map_err(|_| SaveError::InvalidId)?;
        if value == [0; 16] {
            return Err(SaveError::InvalidId);
        }
        Ok(Self(value))
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BranchId([u8; 16]);
impl BranchId {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        let value: [u8; 16] = bytes.try_into().map_err(|_| SaveError::InvalidId)?;
        if value == [0; 16] {
            return Err(SaveError::InvalidId);
        }
        Ok(Self(value))
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveError {
    InvalidId,
    OperationConflict,
    SavePointNotFound,
    NoSavePoint,
    NotMember,
    FenceNotIncreasing,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointEnvelope {
    pub bytes: Vec<u8>,
    pub codec_version: u32,
    pub schema_version: u32,
    pub content_version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveLinkKind {
    FinanceAdmission,
    PaidGeneration,
}
impl SaveLinkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FinanceAdmission => "finance_admission",
            Self::PaidGeneration => "paid_generation",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveLink {
    pub kind: SaveLinkKind,
    pub linked_id: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavePoint {
    pub save_point_id: SavePointId,
    pub tenant_id: TenantId,
    pub campaign_id: CampaignId,
    pub branch_id: BranchId,
    pub parent_save_point_id: Option<SavePointId>,
    pub checkpoint: CheckpointEnvelope,
    pub links: Vec<SaveLink>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateSavePoint {
    pub save_point_id: SavePointId,
    pub tenant_id: TenantId,
    pub campaign_id: CampaignId,
    pub branch_id: BranchId,
    pub parent_save_point_id: Option<SavePointId>,
    pub checkpoint: CheckpointEnvelope,
    pub links: Vec<SaveLink>,
    pub operation_id: OperationId,
    /// Caller-computed fingerprint of the exact request; a retry must match it.
    pub fingerprint: [u8; 32],
    /// The stored result returned to the caller and to every retry.
    pub result_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredResult {
    pub save_point_id: SavePointId,
    pub result_bytes: Vec<u8>,
    /// True when this call committed; false when a retry returned the stored result.
    pub committed_now: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerFence {
    pub recovery_epoch: u64,
    pub claim_seq: u64,
    pub placement_id: [u8; 16],
}

/// Membership is re-checked at use (decision 24); no session credential is an input.
pub trait MembershipChecker {
    fn is_current_member(
        &self,
        tenant_id: TenantId,
        campaign_id: CampaignId,
        member_id: MemberId,
        now_seconds: u64,
    ) -> bool;
}

/// Claims a strictly increasing new owner fence for a resume.
pub trait FenceClaimer {
    fn claim_new_fence(
        &mut self,
        tenant_id: TenantId,
        campaign_id: CampaignId,
    ) -> Result<OwnerFence, SaveError>;
}

/// A clock the caller controls; wall time never enters the store.
pub trait SaveClock {
    fn now_seconds(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ManualClock {
    pub now_seconds: u64,
}
impl ManualClock {
    pub fn advance(&mut self, seconds: u64) {
        self.now_seconds = self.now_seconds.saturating_add(seconds);
    }
}
impl SaveClock for ManualClock {
    fn now_seconds(&self) -> u64 {
        self.now_seconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchRequest {
    pub tenant_id: TenantId,
    pub campaign_id: CampaignId,
    pub source_save_point_id: SavePointId,
    pub new_save_point_id: SavePointId,
    pub new_branch_id: BranchId,
    pub operation_id: OperationId,
    pub fingerprint: [u8; 32],
    pub result_bytes: Vec<u8>,
}

pub trait SaveStore {
    fn create_save_point(&mut self, request: CreateSavePoint) -> Result<StoredResult, SaveError>;
    fn branch_from(&mut self, request: BranchRequest) -> Result<StoredResult, SaveError>;
    fn latest_save_point(
        &self,
        tenant_id: TenantId,
        campaign_id: CampaignId,
        branch_id: BranchId,
    ) -> Option<SavePoint>;
    fn save_point(&self, save_point_id: SavePointId) -> Option<SavePoint>;
    fn save_point_count(&self) -> usize;
}

/// In-memory store: the faithful fake the acceptance suite drives, with the same
/// commit-before-reply, branch and link semantics as the PostgreSQL writer.
#[derive(Default)]
pub struct MemorySaveStore {
    points: BTreeMap<SavePointId, SavePoint>,
    heads: BTreeMap<(TenantId, CampaignId, BranchId), SavePointId>,
    operations: BTreeMap<(TenantId, CampaignId, OperationId), ([u8; 32], StoredResult)>,
    order: Vec<SavePointId>,
}

impl MemorySaveStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn stored_retry(
        &self,
        request_key: (TenantId, CampaignId, OperationId),
        fingerprint: [u8; 32],
    ) -> Result<Option<StoredResult>, SaveError> {
        match self.operations.get(&request_key) {
            Some((stored_fingerprint, stored)) if *stored_fingerprint == fingerprint => {
                Ok(Some(StoredResult {
                    committed_now: false,
                    ..stored.clone()
                }))
            }
            Some(_) => Err(SaveError::OperationConflict),
            None => Ok(None),
        }
    }

    fn insert_point(&mut self, point: SavePoint, operation: CreateSavePoint) {
        let key = (point.tenant_id, point.campaign_id, point.branch_id);
        self.heads.insert(key, point.save_point_id);
        self.order.push(point.save_point_id);
        self.operations.insert(
            (point.tenant_id, point.campaign_id, operation.operation_id),
            (
                operation.fingerprint,
                StoredResult {
                    save_point_id: point.save_point_id,
                    result_bytes: operation.result_bytes.clone(),
                    committed_now: true,
                },
            ),
        );
        self.points.insert(point.save_point_id, point);
    }
}

impl SaveStore for MemorySaveStore {
    fn create_save_point(&mut self, request: CreateSavePoint) -> Result<StoredResult, SaveError> {
        let key = (request.tenant_id, request.campaign_id, request.operation_id);
        if let Some(stored) = self.stored_retry(key, request.fingerprint)? {
            return Ok(stored);
        }
        let point = SavePoint {
            save_point_id: request.save_point_id,
            tenant_id: request.tenant_id,
            campaign_id: request.campaign_id,
            branch_id: request.branch_id,
            parent_save_point_id: request.parent_save_point_id,
            checkpoint: request.checkpoint.clone(),
            links: request.links.clone(),
        };
        let result = StoredResult {
            save_point_id: point.save_point_id,
            result_bytes: request.result_bytes.clone(),
            committed_now: true,
        };
        self.insert_point(point, request);
        Ok(result)
    }

    fn branch_from(&mut self, request: BranchRequest) -> Result<StoredResult, SaveError> {
        let key = (request.tenant_id, request.campaign_id, request.operation_id);
        if let Some(stored) = self.stored_retry(key, request.fingerprint)? {
            return Ok(stored);
        }
        let source = self
            .points
            .get(&request.source_save_point_id)
            .filter(|point| {
                point.tenant_id == request.tenant_id && point.campaign_id == request.campaign_id
            })
            .ok_or(SaveError::SavePointNotFound)?;
        // The branch copies checkpoint bytes and forward-only links; no finance input exists.
        let point = SavePoint {
            save_point_id: request.new_save_point_id,
            tenant_id: request.tenant_id,
            campaign_id: request.campaign_id,
            branch_id: request.new_branch_id,
            parent_save_point_id: Some(request.source_save_point_id),
            checkpoint: source.checkpoint.clone(),
            links: source.links.clone(),
        };
        let operation = CreateSavePoint {
            save_point_id: request.new_save_point_id,
            tenant_id: request.tenant_id,
            campaign_id: request.campaign_id,
            branch_id: request.new_branch_id,
            parent_save_point_id: Some(request.source_save_point_id),
            checkpoint: point.checkpoint.clone(),
            links: point.links.clone(),
            operation_id: request.operation_id,
            fingerprint: request.fingerprint,
            result_bytes: request.result_bytes,
        };
        let result = StoredResult {
            save_point_id: request.new_save_point_id,
            result_bytes: operation.result_bytes.clone(),
            committed_now: true,
        };
        self.insert_point(point, operation);
        Ok(result)
    }

    fn latest_save_point(
        &self,
        tenant_id: TenantId,
        campaign_id: CampaignId,
        branch_id: BranchId,
    ) -> Option<SavePoint> {
        let head = self.heads.get(&(tenant_id, campaign_id, branch_id))?;
        self.points.get(head).cloned()
    }

    fn save_point(&self, save_point_id: SavePointId) -> Option<SavePoint> {
        self.points.get(&save_point_id).cloned()
    }

    fn save_point_count(&self) -> usize {
        self.points.len()
    }
}

/// Monotonic in-memory fence claimer (the fake for O1's directory fence).
#[derive(Default)]
pub struct MemoryFenceClaimer {
    next_claim_seq: BTreeMap<(TenantId, CampaignId), u64>,
    pub recovery_epoch: u64,
    pub placement_id: [u8; 16],
}

impl FenceClaimer for MemoryFenceClaimer {
    fn claim_new_fence(
        &mut self,
        tenant_id: TenantId,
        campaign_id: CampaignId,
    ) -> Result<OwnerFence, SaveError> {
        let entry = self
            .next_claim_seq
            .entry((tenant_id, campaign_id))
            .or_insert(0);
        *entry = entry.checked_add(1).ok_or(SaveError::FenceNotIncreasing)?;
        Ok(OwnerFence {
            recovery_epoch: if self.recovery_epoch == 0 {
                1
            } else {
                self.recovery_epoch
            },
            claim_seq: *entry,
            placement_id: if self.placement_id == [0; 16] {
                [9; 16]
            } else {
                self.placement_id
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeGrant {
    pub fence: OwnerFence,
    pub save_point: SavePoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeRequest {
    pub tenant_id: TenantId,
    pub campaign_id: CampaignId,
    pub member_id: MemberId,
    pub branch_id: BranchId,
}

/// SAVE-01: re-checks membership at use and takes a NEW owner fence. Expired session
/// credentials are not an input, so they cannot block a current member; a revoked
/// member is refused before any fence is claimed.
pub fn resume<S: SaveStore, M: MembershipChecker, F: FenceClaimer, C: SaveClock>(
    store: &S,
    membership: &M,
    fences: &mut F,
    clock: &C,
    request: ResumeRequest,
) -> Result<ResumeGrant, SaveError> {
    if !membership.is_current_member(
        request.tenant_id,
        request.campaign_id,
        request.member_id,
        clock.now_seconds(),
    ) {
        return Err(SaveError::NotMember);
    }
    let save_point = store
        .latest_save_point(request.tenant_id, request.campaign_id, request.branch_id)
        .ok_or(SaveError::NoSavePoint)?;
    let fence = fences.claim_new_fence(request.tenant_id, request.campaign_id)?;
    Ok(ResumeGrant { fence, save_point })
}

#[cfg(not(target_arch = "wasm32"))]
mod postgres;
#[cfg(not(target_arch = "wasm32"))]
pub use postgres::PostgresSaveStore;
