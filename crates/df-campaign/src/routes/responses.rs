//! Responses: JSON bodies, the refusal-to-status table and which refusals end a display's
//! pairing (and so clear its cookies).

use axum::{body::Body, response::Response};
use df_campaign_wire as wire;
use http::{HeaderValue, StatusCode, header};
use serde::Serialize;

use super::cookies::{DEVICE_COOKIE, TICKET_COOKIE, clear_cookie};
use super::guard::GuardRefusal;
use crate::{
    display_pairing::{PairingRefusal, TicketRefusal},
    errors::{Failure, Refusal, StoreError},
    join_invitation::JoinFailure,
};

/// A refusal is a whole HTTP response; it is boxed so the `Err` side of every handler result stays
/// small.
pub(super) type Failed = Box<Response>;
pub(super) type Handled = Result<Response, Failed>;

pub(super) fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response {
    // A body of these types always serializes; the fallback is an empty object, never a success.
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) fn ok<T: Serialize>(body: &T) -> Handled {
    Ok(json_response(StatusCode::OK, body))
}

pub(super) fn reason_response(status: StatusCode, reason: wire::Reason) -> Response {
    json_response(status, &wire::ReasonBody { reason })
}

pub(super) fn refused(status: StatusCode, reason: wire::Reason) -> Failed {
    Box::new(reason_response(status, reason))
}

/// The status and reason a refusal is shown as.
pub fn refusal_reason(refusal: Refusal) -> (StatusCode, wire::Reason) {
    use wire::Reason as R;
    match refusal {
        Refusal::NotFound => (StatusCode::NOT_FOUND, R::NotFound),
        Refusal::NotAllowed | Refusal::OwnerOnly => (StatusCode::FORBIDDEN, R::NotAllowed),
        Refusal::ConfirmationRequired => (StatusCode::BAD_REQUEST, R::ConfirmationRequired),
        Refusal::Join(failure) => join_reason(failure),
        Refusal::InvitationUnknown => (StatusCode::NOT_FOUND, R::InvitationUnknown),
        Refusal::CodeUnknown => (StatusCode::NOT_FOUND, R::CodeUnknown),
        Refusal::AllowBackFirst => (StatusCode::CONFLICT, R::AllowBackFirst),
        Refusal::RequestsFull => (StatusCode::CONFLICT, R::RequestsFull),
        Refusal::StaleRevision => (StatusCode::CONFLICT, R::StaleRevision),
        Refusal::CampaignUnavailable => (StatusCode::CONFLICT, R::CampaignUnavailable),
        Refusal::AccountClosing => (StatusCode::CONFLICT, R::AccountClosing),
        Refusal::SessionInProgress => (StatusCode::CONFLICT, R::SessionInProgress),
        Refusal::SessionEnded => (StatusCode::CONFLICT, R::SessionEnded),
        Refusal::HostAccountNotActive => (StatusCode::CONFLICT, R::HostAccountNotActive),
        Refusal::OwnerAccountNotActive => (StatusCode::CONFLICT, R::OwnerAccountNotActive),
        Refusal::HostMustTransferFirst => (StatusCode::CONFLICT, R::HostMustTransferFirst),
        Refusal::OwnerMustTransferFirst => (StatusCode::CONFLICT, R::OwnerMustTransferFirst),
        Refusal::CannotRemoveHost => (StatusCode::FORBIDDEN, R::CannotRemoveHost),
        Refusal::CannotRemoveOwner => (StatusCode::FORBIDDEN, R::CannotRemoveOwner),
        Refusal::TargetOutranks => (StatusCode::FORBIDDEN, R::TargetOutranks),
        Refusal::DeletionScheduled => (StatusCode::CONFLICT, R::DeletionScheduled),
        Refusal::AlreadyDeleting => (StatusCode::CONFLICT, R::AlreadyDeleting),
        Refusal::DisplayLimit | Refusal::Pairing(PairingRefusal::DisplayLimit) => {
            (StatusCode::CONFLICT, R::DisplayLimit)
        }
        Refusal::DisplayNeutral | Refusal::Pairing(_) | Refusal::Ticket(_) => {
            (StatusCode::FORBIDDEN, R::DisplayNeutral)
        }
        Refusal::OperationConflict => (StatusCode::CONFLICT, R::OperationConflict),
        Refusal::OperationExpired => (StatusCode::GONE, R::OperationExpired),
        Refusal::ProposalExpired => (StatusCode::GONE, R::ProposalExpired),
        Refusal::InvalidInput => (StatusCode::BAD_REQUEST, R::InvalidInput),
        Refusal::SeatsFull => (StatusCode::CONFLICT, R::SeatsFull),
        Refusal::ContributorsFull => (StatusCode::CONFLICT, R::ContributorsFull),
        Refusal::NotSeated => (StatusCode::CONFLICT, R::NotSeated),
        Refusal::ContributorCannotBeCoHost => (StatusCode::CONFLICT, R::ContributorCannotBeCoHost),
        Refusal::TransferTargetNotAllowed => (StatusCode::CONFLICT, R::TransferTargetNotAllowed),
        Refusal::TooManyCampaigns => (StatusCode::CONFLICT, R::TooManyCampaigns),
        Refusal::Throttled => (StatusCode::TOO_MANY_REQUESTS, R::Throttled),
    }
}

fn join_reason(failure: JoinFailure) -> (StatusCode, wire::Reason) {
    use wire::Reason as R;
    match failure {
        JoinFailure::Expired => (StatusCode::GONE, R::InvitationExpired),
        JoinFailure::Revoked => (StatusCode::GONE, R::InvitationRevoked),
        JoinFailure::Exhausted => (StatusCode::GONE, R::InvitationExhausted),
        JoinFailure::Full => (StatusCode::CONFLICT, R::Full),
        JoinFailure::ContributorsFull => (StatusCode::CONFLICT, R::ContributorsFull),
        JoinFailure::AlreadyJoined => (StatusCode::CONFLICT, R::AlreadyJoined),
        JoinFailure::NoLongerValid => (StatusCode::GONE, R::NoLongerValid),
        JoinFailure::CampaignUnavailable => (StatusCode::CONFLICT, R::CampaignUnavailable),
        JoinFailure::AccountClosing => (StatusCode::CONFLICT, R::AccountClosing),
    }
}

pub fn failure_reason(failure: Failure) -> (StatusCode, wire::Reason) {
    use wire::Reason as R;
    match failure {
        Failure::Refused(refusal) => refusal_reason(refusal),
        Failure::Store(StoreError::ServiceHalting) => {
            (StatusCode::SERVICE_UNAVAILABLE, R::ServiceHalting)
        }
        Failure::Store(StoreError::Unknown) => (StatusCode::SERVICE_UNAVAILABLE, R::OutcomeUnknown),
        Failure::Store(
            StoreError::Unavailable
            | StoreError::Retryable
            | StoreError::InvalidRecord
            | StoreError::FenceStale
            | StoreError::TimeoutNotSet
            | StoreError::RandomUnavailable,
        ) => (StatusCode::SERVICE_UNAVAILABLE, R::Unavailable),
    }
}

pub(super) fn failure_response(failure: Failure) -> Failed {
    let (status, reason) = failure_reason(failure);
    Box::new(reason_response(status, reason))
}

/// Whether a refusal ends the display's pairing, so both display cookies are cleared. Only the
/// terminal ones do: the pairing is expired, cancelled, unpaired or revoked, the device is wrong or
/// unknown, or the session ended. A refusal that leaves a good session standing (the cap, a
/// renewal that is too early or past its cap, a repeat of a redemption that already worked, a
/// throttle) keeps the cookies, so a display that renews early is not signed out.
pub fn ends_display_pairing(failure: Failure) -> bool {
    match failure {
        Failure::Refused(Refusal::DisplayNeutral | Refusal::SessionEnded) => true,
        Failure::Refused(Refusal::Pairing(refusal)) => matches!(
            refusal,
            PairingRefusal::WrongDevice
                | PairingRefusal::Expired
                | PairingRefusal::Cancelled
                | PairingRefusal::ApproverNotAllowed
        ),
        Failure::Refused(Refusal::Ticket(refusal)) => matches!(
            refusal,
            TicketRefusal::Revoked
                | TicketRefusal::WrongDevice
                | TicketRefusal::Expired
                | TicketRefusal::ApproverLostAuthority
                | TicketRefusal::SessionEnded
        ),
        Failure::Refused(_) | Failure::Store(_) => false,
    }
}

/// A display flow refusal; it clears both display cookies only when it ends the pairing.
pub(super) fn display_failure_response(failure: Failure) -> Failed {
    let ends = ends_display_pairing(failure);
    let mut response = failure_response(failure);
    if ends {
        append_cookie(&mut response, &clear_cookie(DEVICE_COOKIE));
        append_cookie(&mut response, &clear_cookie(TICKET_COOKIE));
    }
    response
}

pub(super) fn guard_response(refusal: GuardRefusal) -> Response {
    *match refusal {
        GuardRefusal::Unauthenticated => {
            refused(StatusCode::UNAUTHORIZED, wire::Reason::Unauthenticated)
        }
        GuardRefusal::CsrfRefused => refused(StatusCode::FORBIDDEN, wire::Reason::CsrfRefused),
        GuardRefusal::Unavailable => {
            refused(StatusCode::SERVICE_UNAVAILABLE, wire::Reason::Unavailable)
        }
    }
}

pub(super) fn append_cookie(response: &mut Response, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}
