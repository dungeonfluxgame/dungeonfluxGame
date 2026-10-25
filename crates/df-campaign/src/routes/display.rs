//! Handlers for display pairing: the device routes (challenge, status, redeem, renew) and the
//! member side (lookup, approve, unpair).

use df_campaign_wire as wire;
use http::{HeaderMap, StatusCode, header};

use super::{
    cookies::{
        DEVICE_COOKIE, TICKET_COOKIE, check_display_request, clear_cookie, cookie_value, set_cookie,
    },
    guard::Guard,
    request::{Ctx, hex_of, invalid},
    responses::{
        Failed, Handled, append_cookie, display_failure_response, failure_response, json_response,
        ok, refused,
    },
};
use crate::{
    display_pairing::PairingState,
    errors::{Failure, Refusal, StoreError},
    ids::{PairingId, TicketId},
    records::DeviceLabel,
    service::{Ports, pairing_state_name},
    short_code::ShortCode,
};

fn label_of(headers: &HeaderMap) -> DeviceLabel {
    // The same coarse classification as B's session device labels: a hint, never a decision.
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if agent.contains("ipad") || agent.contains("tablet") {
        DeviceLabel::Tablet
    } else if agent.contains("mobile") || agent.contains("iphone") || agent.contains("android") {
        DeviceLabel::Phone
    } else if agent.is_empty() {
        DeviceLabel::Unknown
    } else {
        DeviceLabel::Desktop
    }
}

/// The user code as the display shows it (`ABCD-EFGH`). A stored code that does not parse is a
/// corrupt record, never an empty code in a success.
fn display_code(stored: Option<&str>) -> Result<String, Failure> {
    stored
        .and_then(|code| ShortCode::parse(code).ok())
        .map(|code| code.display_form())
        .ok_or(Failure::Store(StoreError::InvalidRecord))
}

pub(super) async fn challenge<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let service = ctx.service();
    let challenge = service
        .begin_display_pairing(ctx.caller.client_ip, label_of(&ctx.parts.headers))
        .await
        .map_err(failure_response)?;
    let max_age = u64::from(service.tunables().display_pairing_maximum_hours) * 3_600;
    let user_code =
        display_code(challenge.pairing.user_code.as_deref()).map_err(failure_response)?;
    let mut response = json_response(
        StatusCode::OK,
        &wire::DisplayChallengeResponse {
            user_code,
            expires_at_ms: challenge.pairing.expires_at_ms,
        },
    );
    append_cookie(
        &mut response,
        &set_cookie(DEVICE_COOKIE, &challenge.device_secret, max_age),
    );
    Ok(response)
}

pub(super) async fn status<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let Some(secret) = cookie_value(&ctx.parts.headers, DEVICE_COOKIE) else {
        return Err(display_failure_response(Failure::Refused(
            Refusal::DisplayNeutral,
        )));
    };
    let pairing = ctx
        .service()
        .display_status(ctx.caller.client_ip, &secret)
        .await
        .map_err(display_failure_response)?;
    let terminal = matches!(
        pairing.state,
        PairingState::Cancelled | PairingState::Expired
    );
    let user_code = match (pairing.state, pairing.user_code.as_deref()) {
        (PairingState::Pending, code) => Some(display_code(code).map_err(failure_response)?),
        _ => None,
    };
    let mut response = json_response(
        StatusCode::OK,
        &wire::DisplayStatusResponse {
            state: pairing_state_name(pairing.state),
            user_code,
        },
    );
    if terminal {
        append_cookie(&mut response, &clear_cookie(DEVICE_COOKIE));
        append_cookie(&mut response, &clear_cookie(TICKET_COOKIE));
    }
    Ok(response)
}

/// Redeem and renew take no CSRF token: they rely on the checks of [`check_display_request`].
fn gate<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Result<String, Failed> {
    if check_display_request(&ctx.parts.headers, &ctx.app.origin).is_err() {
        return Err(refused(StatusCode::FORBIDDEN, wire::Reason::CsrfRefused));
    }
    cookie_value(&ctx.parts.headers, DEVICE_COOKIE)
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, wire::Reason::CsrfRefused))
}

pub(super) async fn redeem<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let device = gate(ctx)?;
    let ticket_cookie = cookie_value(&ctx.parts.headers, TICKET_COOKIE);
    let service = ctx.service();
    let redeemed = service
        .redeem_display_ticket(ctx.caller.client_ip, &device, ticket_cookie.as_deref())
        .await
        .map_err(display_failure_response)?;
    let ticket = &redeemed.redeemed.ticket;
    let remaining = ticket.expires_at_ms.saturating_sub(service.now_ms()) / 1_000;
    let mut response = json_response(
        StatusCode::OK,
        &wire::DisplayRedeemResponse {
            session_id: hex_of(ticket.session.as_bytes()),
            ticket_expires_at_ms: ticket.expires_at_ms,
        },
    );
    append_cookie(
        &mut response,
        &set_cookie(TICKET_COOKIE, &redeemed.ticket_secret, remaining),
    );
    Ok(response)
}

pub(super) async fn renew<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let device = gate(ctx)?;
    let Some(ticket_cookie) = cookie_value(&ctx.parts.headers, TICKET_COOKIE) else {
        return Err(display_failure_response(Failure::Refused(
            Refusal::DisplayNeutral,
        )));
    };
    let service = ctx.service();
    let ticket = service
        .extend_display_ticket(ctx.caller.client_ip, &device, &ticket_cookie)
        .await
        .map_err(display_failure_response)?;
    let remaining = ticket.expires_at_ms.saturating_sub(service.now_ms()) / 1_000;
    let mut response = json_response(
        StatusCode::OK,
        &wire::DisplayRenewResponse {
            ticket_expires_at_ms: ticket.expires_at_ms,
        },
    );
    append_cookie(
        &mut response,
        &set_cookie(TICKET_COOKIE, &ticket_cookie, remaining),
    );
    Ok(response)
}

pub(super) async fn lookup<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::DisplayLookupBody = ctx.json()?;
    let found = ctx
        .service()
        .lookup_display_code(
            ctx.caller.client_ip,
            ctx.account()?,
            ctx.campaign()?,
            &body.code,
        )
        .await
        .map_err(failure_response)?;
    ok(&found)
}

pub(super) async fn approve<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::DisplayApproveBody = ctx.json()?;
    let pairing = PairingId::parse(&body.pairing_id).ok_or_else(invalid)?;
    ctx.service()
        .approve_display_pairing(ctx.account()?, ctx.campaign()?, ctx.session()?, pairing)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn unpair<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::UnpairBody = ctx.json()?;
    let ticket = match body.ticket_id {
        Some(text) => Some(TicketId::parse(&text).ok_or_else(invalid)?),
        None => None,
    };
    let revoked = ctx
        .service()
        .unpair_display(ctx.account()?, ctx.campaign()?, ctx.session()?, ticket)
        .await
        .map_err(failure_response)?;
    ok(&wire::UnpairResponse { revoked })
}
