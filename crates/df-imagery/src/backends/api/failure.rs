//! Private bounded protocol observations; rejection never asserts zero supplier liability.
use super::{ApiError, NativeHttpRefusal};
use df_providers::{NativeHttpResponse, ProviderRequestId, TextSchemaLimits, parse_unique_json};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApiExchange {
    Submit,
    Poll,
    Result,
    Delivery,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeclineContract {
    OpenAiImages429V1,
    FalQueue429V1,
    BflImages429V1,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmissionEvidence {
    BeforeTransport,
    RejectedSubmit(DeclineContract),
    AcceptedJob,
    Unknown,
}
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ApiResponseReceipt {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) transport_request_id: Option<ProviderRequestId>,
    pub(crate) retry_after: df_providers::NativeRetryAfter,
}
impl std::fmt::Debug for ApiResponseReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiResponseReceipt")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .field(
                "has_transport_request_id",
                &self.transport_request_id.is_some(),
            )
            .finish()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApiFailure {
    pub(crate) error: ApiError,
    pub(crate) exchange: ApiExchange,
    pub(crate) submission: SubmissionEvidence,
    pub(crate) response: Option<ApiResponseReceipt>,
}
impl ApiFailure {
    pub(super) fn before_transport(error: ApiError) -> Self {
        Self {
            error,
            exchange: ApiExchange::Submit,
            submission: SubmissionEvidence::BeforeTransport,
            response: None,
        }
    }
    pub(super) fn unknown(error: ApiError, exchange: ApiExchange) -> Self {
        Self {
            error,
            exchange,
            submission: SubmissionEvidence::Unknown,
            response: None,
        }
    }
    pub(super) fn accepted(error: ApiError, exchange: ApiExchange) -> Self {
        Self {
            error,
            exchange,
            submission: SubmissionEvidence::AcceptedJob,
            response: None,
        }
    }
    pub(super) fn transport(reason: NativeHttpRefusal, exchange: ApiExchange) -> Self {
        let submission = if exchange != ApiExchange::Submit {
            SubmissionEvidence::AcceptedJob
        } else if matches!(
            reason,
            NativeHttpRefusal::InvalidLimits
                | NativeHttpRefusal::InvalidConfiguration
                | NativeHttpRefusal::InvalidEndpoint
                | NativeHttpRefusal::InvalidCredential
                | NativeHttpRefusal::ProviderDisabled
                | NativeHttpRefusal::RequestTooLarge
        ) {
            SubmissionEvidence::BeforeTransport
        } else {
            SubmissionEvidence::Unknown
        };
        Self {
            error: ApiError::Transport(reason),
            exchange,
            submission,
            response: None,
        }
    }
    pub(super) fn response(
        error: ApiError,
        exchange: ApiExchange,
        contract: DeclineContract,
        response: NativeHttpResponse,
    ) -> Self {
        let submission = if exchange != ApiExchange::Submit {
            SubmissionEvidence::AcceptedJob
        } else if error == ApiError::RateLimited
            && validate_submit_decline(contract, response.status, &response.body)
        {
            SubmissionEvidence::RejectedSubmit(contract)
        } else {
            SubmissionEvidence::Unknown
        };
        Self {
            error,
            exchange,
            submission,
            response: Some(ApiResponseReceipt {
                status: response.status,
                body: response.body,
                transport_request_id: response.transport_request_id,
                retry_after: response.metadata.retry_after,
            }),
        }
    }
    pub(crate) fn retryable_submit_decline(&self) -> Option<DeclineContract> {
        let SubmissionEvidence::RejectedSubmit(contract) = self.submission else {
            return None;
        };
        let response = self.response.as_ref()?;
        (self.error == ApiError::RateLimited
            && self.exchange == ApiExchange::Submit
            && response
                .retry_after
                .minimum_delay(std::time::SystemTime::now())
                .is_some()
            && validate_submit_decline(contract, response.status, &response.body))
        .then_some(contract)
    }
}
/// Replays only documented submit rejection shapes; it makes no billing claim.
/// Official contracts and their exact scope are retained in decline-receipt-a1/contract.md.
pub(crate) fn validate_submit_decline(contract: DeclineContract, status: u16, body: &[u8]) -> bool {
    if status != 429 || body.is_empty() {
        return false;
    }
    let Ok(value) = parse_unique_json(
        body,
        TextSchemaLimits {
            maximum_schema_bytes: body.len(),
            maximum_depth: 32,
            maximum_nodes: 65_536,
        },
    ) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    match contract {
        DeclineContract::OpenAiImages429V1 => {
            if object.len() != 1 {
                return false;
            }
            let Some(error) = object.get("error").and_then(Value::as_object) else {
                return false;
            };
            if error
                .keys()
                .any(|key| !matches!(key.as_str(), "code" | "type" | "message" | "param"))
                || error.get("message").is_some_and(|value| !value.is_string())
                || error
                    .get("param")
                    .is_some_and(|value| !value.is_null() && !value.is_string())
            {
                return false;
            }
            let kind = match error.get("type") {
                None => None,
                Some(Value::String(kind)) if kind == "rate_limit_error" => Some(kind.as_str()),
                _ => return false,
            };
            match error.get("code").and_then(Value::as_str) {
                Some("rate_limit_exceeded") => true,
                Some("slow_down") => kind == Some("rate_limit_error"),
                _ => false,
            }
        }
        DeclineContract::FalQueue429V1 => {
            object.len() == 2
                && object
                    .get("error")
                    .and_then(Value::as_str)
                    .is_some_and(|error| !error.trim().is_empty())
                && object.get("request_id").is_some_and(Value::is_null)
        }
        // Official BFL docs specify status429 but no closed nonacceptance body contract.
        DeclineContract::BflImages429V1 => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_declines_are_distinct_protocol_shapes_and_never_zero_charge_claims() {
        assert!(validate_submit_decline(
            DeclineContract::OpenAiImages429V1,
            429,
            br#"{"error":{"code":"rate_limit_exceeded"}}"#
        ));
        assert!(validate_submit_decline(DeclineContract::OpenAiImages429V1,429,
            br#"{"error":{"code":"slow_down","type":"rate_limit_error","message":"diagnostic","param":null}}"#));
        assert!(validate_submit_decline(
            DeclineContract::FalQueue429V1,
            429,
            br#"{"error":"Queue rejected this request","request_id":null}"#
        ));
        for contract in [
            DeclineContract::OpenAiImages429V1,
            DeclineContract::FalQueue429V1,
            DeclineContract::BflImages429V1,
        ] {
            for body in [br#"{"error":{}}"#.as_slice(),br#"{"detail":{}}"#,
                br#"{"detail":"too many requests"}"#,
                br#"{"error":{"code":"insufficient_quota"}}"#,
                br#"{"error":{"code":"rate_limit_exceeded","accepted":{"job_id":"paid"}}}"#,
                br#"{"error":{"code":"rate_limit_exceeded","message":{"job_id":"paid"}}}"#,
                br#"{"error":{"code":"rate_limit_exceeded"},"job_id":"paid"}"#,
                br#"{"error":{"code":"rate_limit_exceeded","type":4}}"#,
                br#"{"error":{"code":"unknown"}}"#,
                br#"{"error":{"code":"slow_down"}}"#,
                br#"{"error":{"code":"slow_down","type":"insufficient_quota"}}"#,
                br#"{"error":{"code":"rate_limit_exceeded"},"error":{"code":"rate_limit_exceeded"}}"#,
                br#"{"error":"429","request_id":"paid"}"#,
                br#"{"error":"429","request_id":null,"data":{"request_id":"paid"}}"#,
                br#"{"error":{"job_id":"paid"},"request_id":null}"#,
                br#"{"error":"","request_id":null}"#,
                br#"{"error":"429"}"#,b"not json",b"{}"] {
                assert!(!validate_submit_decline(contract,429,body),"{contract:?}: {body:?}");
            }
            assert!(!validate_submit_decline(
                contract,
                503,
                br#"{"error":{"code":"rate_limit_exceeded"}}"#
            ));
        }
        assert!(!validate_submit_decline(
            DeclineContract::BflImages429V1,
            429,
            br#"{"error":{"code":"rate_limit_exceeded"}}"#
        ));
        assert!(!validate_submit_decline(DeclineContract::FalQueue429V1,429,
            br#"{"type":"concurrent_requests_limit","detail":"limit","data":{"limit":2,"notify":true}}"#));
    }
    #[test]
    fn transport_unknown_deadline_and_accepted_poll_have_no_fresh_submit_authority() {
        for reason in [
            NativeHttpRefusal::Unknown,
            NativeHttpRefusal::Deadline,
            NativeHttpRefusal::Cancelled,
        ] {
            let failure = ApiFailure::transport(reason, ApiExchange::Submit);
            assert_eq!(failure.submission, SubmissionEvidence::Unknown);
            assert_eq!(failure.retryable_submit_decline(), None);
        }
        let failure = ApiFailure::accepted(ApiError::RateLimited, ApiExchange::Poll);
        assert_eq!(failure.retryable_submit_decline(), None);
    }
}
