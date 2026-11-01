//! A fixed ranking epoch lets bounded queue pages continue fairly while actual eligibility changes.
use super::{ExecutionStageId, QueuedStage};
use crate::ImageJobId;
/// Exact last-row key within one queue traversal. It grants no claim or send authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCursor {
    pub ranking_at_ms: i64,
    pub effective_urgency: u8,
    pub created_at_ms: i64,
    pub deadline_ms: i64,
    pub stage: ExecutionStageId,
    pub job: ImageJobId,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QueuePage {
    pub stages: Vec<QueuedStage>,
    /// Continue this fixed ranking epoch; None wraps the next traversal to a fresh epoch.
    pub next: Option<QueueCursor>,
}
