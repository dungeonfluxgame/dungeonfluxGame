//! Acknowledgment requires the actual durable Book tombstone and the exact output seal.
use super::ExecutionScope;
use crate::cost::DurableUnsentReceipt;
use crate::publication::NoDeliveryReceipt;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcknowledgeUnsentProjection {
    pub scope: ExecutionScope,
    pub delivery: NoDeliveryReceipt,
    pub projection: DurableUnsentReceipt,
    pub now_ms: i64,
}
