//! Unit tests of the route table, the display request checks, the cookies and the refusal table.

use df_campaign_wire as wire;
use http::{HeaderMap, HeaderValue, StatusCode, header};

use super::{
    AuthClass, DEVICE_COOKIE, DEVICE_HEADER, DisplayRequestRefusal, Gate, ROUTES, RouteId,
    TICKET_COOKIE, check_display_request, clear_cookie, cookie_value, ends_display_pairing,
    failure_reason, refusal_reason, set_cookie,
};
use crate::{
    display_pairing::{PairingRefusal, TicketRefusal},
    errors::{Failure, Refusal, StoreError},
    join_invitation::JoinFailure,
    roles::Capability,
};

const ORIGIN: &str = "https://play.example.test";

fn headers(extra: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    let base = [
        ("origin", ORIGIN),
        ("sec-fetch-site", "same-origin"),
        (DEVICE_HEADER, "1"),
        ("content-type", "application/json"),
        ("cookie", "__Host-df_display_device=abc; other=1"),
    ];
    for (name, value) in base.iter().chain(extra.iter()) {
        map.insert(
            http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn without(name: &str) -> HeaderMap {
    let mut map = headers(&[]);
    map.remove(name);
    map
}

#[test]
fn a_complete_display_request_passes() {
    assert_eq!(check_display_request(&headers(&[]), ORIGIN), Ok(()));
}

#[test]
fn redeem_and_renew_accept_an_absent_sec_fetch_site_when_origin_x_df_device_and_the_strict_cookie_are_present()
 {
    assert_eq!(
        check_display_request(&without("sec-fetch-site"), ORIGIN),
        Ok(())
    );
}

#[test]
fn redeem_and_renew_refuse_a_present_sec_fetch_site_that_is_not_same_origin() {
    for site in ["same-site", "cross-site", "none", ""] {
        assert_eq!(
            check_display_request(&headers(&[("sec-fetch-site", site)]), ORIGIN),
            Err(DisplayRequestRefusal::CrossSite),
            "{site:?}"
        );
    }
}

#[test]
fn redeem_and_renew_are_refused_without_origin_device_header_json_or_the_cookie() {
    assert_eq!(
        check_display_request(&without("origin"), ORIGIN),
        Err(DisplayRequestRefusal::MissingOrForeignOrigin)
    );
    assert_eq!(
        check_display_request(&headers(&[("origin", "https://evil.example.test")]), ORIGIN),
        Err(DisplayRequestRefusal::MissingOrForeignOrigin)
    );
    assert_eq!(
        check_display_request(&without(DEVICE_HEADER), ORIGIN),
        Err(DisplayRequestRefusal::MissingDeviceHeader)
    );
    assert_eq!(
        check_display_request(&headers(&[("content-type", "text/plain")]), ORIGIN),
        Err(DisplayRequestRefusal::NotJson)
    );
    assert_eq!(
        check_display_request(&without("cookie"), ORIGIN),
        Err(DisplayRequestRefusal::MissingDeviceCookie)
    );
    // A JSON content type with a charset still counts.
    assert_eq!(
        check_display_request(
            &headers(&[("content-type", "application/json; charset=utf-8")]),
            ORIGIN
        ),
        Ok(())
    );
}

#[test]
fn cookies_are_read_by_exact_name_across_headers() {
    let mut map = HeaderMap::new();
    map.append(
        header::COOKIE,
        HeaderValue::from_static("a=1; __Host-df_display_device=secret"),
    );
    map.append(
        header::COOKIE,
        HeaderValue::from_static("__Host-df_display_ticket=t"),
    );
    assert_eq!(cookie_value(&map, DEVICE_COOKIE).as_deref(), Some("secret"));
    assert_eq!(cookie_value(&map, TICKET_COOKIE).as_deref(), Some("t"));
    assert_eq!(cookie_value(&map, "__Host-df_display"), None);
}

#[test]
fn display_cookies_are_secure_http_only_strict_and_carry_their_max_age() {
    let set = set_cookie(DEVICE_COOKIE, "abc", 86_400);
    assert!(set.contains("Max-Age=86400") && set.contains("Secure"));
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Strict") && set.contains("Path=/"));
    assert!(!set.contains("Domain"), "a __Host- cookie has no Domain");
    let cleared = clear_cookie(TICKET_COOKIE);
    assert!(cleared.contains("Max-Age=0"));
}

#[test]
fn route_paths_are_unique_and_every_route_id_has_exactly_one_row() {
    let mut paths: Vec<_> = ROUTES
        .iter()
        .map(|route| (route.path, route.verb as u8))
        .collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), ROUTES.len());
    let mut ids: Vec<_> = ROUTES
        .iter()
        .map(|route| format!("{:?}", route.id))
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), ROUTES.len());
    for route in ROUTES {
        assert!(route.path.starts_with('/'));
    }
}

#[test]
fn every_route_that_changes_a_campaign_names_its_gate() {
    for route in ROUTES {
        if route.auth == AuthClass::Session {
            assert_ne!(route.gate, Gate::NotMemberAction, "{:?}", route.id);
        } else {
            assert_eq!(route.gate, Gate::NotMemberAction, "{:?}", route.id);
        }
    }
    let gated: Vec<_> = ROUTES
        .iter()
        .filter_map(|route| match route.gate {
            Gate::Capability(capability) => Some(capability),
            _ => None,
        })
        .collect();
    assert!(gated.contains(&Capability::InviteOrRemovePlayers));
    assert!(gated.contains(&Capability::StartOrPauseSession));
}

#[test]
fn invitation_accept_and_room_code_routes_require_an_authenticated_session() {
    for id in [
        RouteId::JoinAccept,
        RouteId::CodeLookup,
        RouteId::CodeRequest,
    ] {
        let route = ROUTES
            .iter()
            .find(|route| route.id == id)
            .expect("every route id appears in ROUTES");
        assert_eq!(route.auth, AuthClass::Session, "{id:?}");
    }
}

#[test]
fn every_refusal_and_store_error_has_a_status_and_a_reason() {
    let refusals = [
        Refusal::NotFound,
        Refusal::NotAllowed,
        Refusal::OwnerOnly,
        Refusal::ConfirmationRequired,
        Refusal::InvitationUnknown,
        Refusal::CodeUnknown,
        Refusal::AllowBackFirst,
        Refusal::RequestsFull,
        Refusal::StaleRevision,
        Refusal::CampaignUnavailable,
        Refusal::AccountClosing,
        Refusal::SessionInProgress,
        Refusal::SessionEnded,
        Refusal::HostAccountNotActive,
        Refusal::HostMustTransferFirst,
        Refusal::OwnerMustTransferFirst,
        Refusal::CannotRemoveHost,
        Refusal::CannotRemoveOwner,
        Refusal::TargetOutranks,
        Refusal::DeletionScheduled,
        Refusal::AlreadyDeleting,
        Refusal::DisplayLimit,
        Refusal::DisplayNeutral,
        Refusal::OperationConflict,
        Refusal::OperationExpired,
        Refusal::ProposalExpired,
        Refusal::InvalidInput,
        Refusal::SeatsFull,
        Refusal::OwnerAccountNotActive,
        Refusal::TooManyCampaigns,
        Refusal::EmailUnverified,
        Refusal::DraftsFull,
        Refusal::DraftIncomplete,
        Refusal::CharacterNotChosen,
        Refusal::ContributorsFull,
        Refusal::NotSeated,
        Refusal::ContributorCannotBeCoHost,
        Refusal::TransferTargetNotAllowed,
        Refusal::Throttled,
        Refusal::Join(JoinFailure::Expired),
        Refusal::Join(JoinFailure::Revoked),
        Refusal::Join(JoinFailure::Exhausted),
        Refusal::Join(JoinFailure::Full),
        Refusal::Join(JoinFailure::ContributorsFull),
        Refusal::Join(JoinFailure::AlreadyJoined),
        Refusal::Join(JoinFailure::NoLongerValid),
        Refusal::Join(JoinFailure::CampaignUnavailable),
        Refusal::Join(JoinFailure::AccountClosing),
    ];
    for refusal in refusals {
        let (status, _) = refusal_reason(refusal);
        assert!(status.is_client_error(), "{refusal:?}");
    }
    // A display never learns which part of a pairing failed.
    let neutral = [
        Refusal::Pairing(PairingRefusal::WrongDevice),
        Refusal::Pairing(PairingRefusal::Expired),
        Refusal::Ticket(TicketRefusal::Revoked),
        Refusal::Ticket(TicketRefusal::WrongDevice),
        Refusal::Ticket(TicketRefusal::Expired),
    ];
    for refusal in neutral {
        assert_eq!(refusal_reason(refusal).1, wire::Reason::DisplayNeutral);
    }
    assert_eq!(
        failure_reason(Failure::Store(StoreError::Unknown)),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            wire::Reason::OutcomeUnknown
        )
    );
    assert_eq!(
        failure_reason(Failure::Store(StoreError::ServiceHalting)),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            wire::Reason::ServiceHalting
        )
    );
}

#[test]
fn only_terminal_refusals_clear_the_display_cookies() {
    let clears = [
        Refusal::DisplayNeutral,
        Refusal::SessionEnded,
        Refusal::Pairing(PairingRefusal::WrongDevice),
        Refusal::Pairing(PairingRefusal::Expired),
        Refusal::Pairing(PairingRefusal::Cancelled),
        Refusal::Pairing(PairingRefusal::ApproverNotAllowed),
        Refusal::Ticket(TicketRefusal::Revoked),
        Refusal::Ticket(TicketRefusal::WrongDevice),
        Refusal::Ticket(TicketRefusal::Expired),
        Refusal::Ticket(TicketRefusal::ApproverLostAuthority),
        Refusal::Ticket(TicketRefusal::SessionEnded),
    ];
    for refusal in clears {
        assert!(
            ends_display_pairing(Failure::Refused(refusal)),
            "{refusal:?}"
        );
    }
    let keeps = [
        Refusal::DisplayLimit,
        Refusal::Throttled,
        Refusal::Pairing(PairingRefusal::NotApproved),
        Refusal::Pairing(PairingRefusal::NotPending),
        Refusal::Pairing(PairingRefusal::AlreadyRedeemed),
        Refusal::Pairing(PairingRefusal::ReissueUsed),
        Refusal::Pairing(PairingRefusal::ReissueWindowClosed),
        Refusal::Pairing(PairingRefusal::DisplayLimit),
        Refusal::Ticket(TicketRefusal::OutsideRenewalWindow),
        Refusal::Ticket(TicketRefusal::RenewalCapReached),
        Refusal::NotFound,
    ];
    for refusal in keeps {
        assert!(
            !ends_display_pairing(Failure::Refused(refusal)),
            "{refusal:?}"
        );
    }
    assert!(!ends_display_pairing(Failure::Store(
        StoreError::Unavailable
    )));
}
