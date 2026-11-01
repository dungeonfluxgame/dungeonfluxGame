//! Operational queue notices do not assert a supplier failure or financial outcome.
use super::{ExecutionSetupError, StageScope};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaDeferralReason {
    Cooldown,
    Probing,
}
impl QuotaDeferralReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cooldown => "cooldown",
            Self::Probing => "probing",
        }
    }
    pub fn parse_label(value: &str) -> Option<Self> {
        match value {
            "cooldown" => Some(Self::Cooldown),
            "probing" => Some(Self::Probing),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueDeferralNotice {
    pub reason: QuotaDeferralReason,
    pub retry_at_ms: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferQueuedCommand {
    pub scope: StageScope,
    pub not_before_ms: i64,
    pub notice: Option<QuotaDeferralReason>,
    pub now_ms: i64,
}
impl DeferQueuedCommand {
    pub fn new(
        scope: StageScope,
        not_before_ms: i64,
        notice: Option<QuotaDeferralReason>,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        if now_ms <= 0 || not_before_ms <= now_ms {
            return Err(ExecutionSetupError::BadTime);
        }
        Ok(Self {
            scope,
            not_before_ms,
            notice,
            now_ms,
        })
    }
}
