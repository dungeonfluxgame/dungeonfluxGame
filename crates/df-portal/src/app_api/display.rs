//! Display pairing for the TV (S1 8.5): `challenge` shows a code and sets the Strict device
//! cookie, `status` polls it without side effects, and `redeem` and `renew` use the device guard
//! (no CSRF token) because a TV that sat idle has none that is fresh. The device secret and the
//! ticket live only in `HttpOnly` cookies; a terminal failure clears the device cookie.

use axum::{extract::State, http::HeaderMap};
use df_account_app::{
    wire::{ApiError, ErrorCode},
    wire_campaign::{
        DisplayChallenge, DisplayRedeemed, DisplayRenewed, DisplayState, DisplayStatus, Reason,
    },
};
use df_customer::{AccountStore, EmailSender};

use super::{
    guard::{
        ApiBody, ApiFailure, ApiResult, DEVICE_COOKIE, DevicePost, Nothing, Reply, SameSite,
        TICKET_COOKIE, VisitorPost, clear_secret_cookie, reply, secret_cookie,
    },
    ports::{CampaignDirectory, PortError, Ports},
    state::AppApiState,
};
use crate::security::{ClientInfo, cookie};

/// The neutral failure of every terminal state: expired, cancelled, wrong or missing secret.
fn no_longer_valid() -> ApiFailure {
    ApiFailure::new(ApiError::because(
        ErrorCode::NotFound,
        Reason::NoLongerValid,
    ))
    .with_cookie(clear_secret_cookie(DEVICE_COOKIE, SameSite::Strict))
}

fn device_failure(error: PortError) -> ApiFailure {
    match error {
        PortError::NotFound | PortError::Forbidden | PortError::Because(..) => no_longer_valid(),
        other => ApiFailure::from(other),
    }
}

pub(super) async fn challenge<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<DisplayChallenge> {
    let issued = state
        .ports
        .campaigns()
        .display_challenge(client.device)
        .await?;
    Ok(Reply::new(issued.challenge).with_cookie(secret_cookie(
        DEVICE_COOKIE,
        &issued.device.secret,
        issued.device.max_age_s,
        SameSite::Strict,
    )))
}

pub(super) async fn status<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    headers: HeaderMap,
) -> ApiResult<DisplayStatus> {
    let Some(secret) = cookie(&headers, DEVICE_COOKIE).filter(|secret| !secret.is_empty()) else {
        return reply(DisplayStatus {
            state: DisplayState::Expired,
        });
    };
    match state.ports.campaigns().display_status(secret).await {
        Ok(display_state) => reply(DisplayStatus {
            state: display_state,
        }),
        Err(PortError::NotFound | PortError::Because(..)) => Ok(Reply::new(DisplayStatus {
            state: DisplayState::Expired,
        })
        .with_cookie(clear_secret_cookie(DEVICE_COOKIE, SameSite::Strict))),
        Err(other) => Err(other.into()),
    }
}

pub(super) async fn redeem<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    device: DevicePost,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<DisplayRedeemed> {
    let issued = state
        .ports
        .campaigns()
        .display_redeem(&device.device_secret)
        .await
        .map_err(device_failure)?;
    Ok(Reply::new(issued.redeemed).with_cookie(secret_cookie(
        TICKET_COOKIE,
        &issued.ticket.secret,
        issued.ticket.max_age_s,
        SameSite::Strict,
    )))
}

pub(super) async fn renew<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    device: DevicePost,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<DisplayRenewed> {
    let ticket = device
        .ticket
        .as_deref()
        .filter(|ticket| !ticket.is_empty())
        .ok_or_else(no_longer_valid)?;
    let renewed = state
        .ports
        .campaigns()
        .display_renew(&device.device_secret, ticket)
        .await
        .map_err(device_failure)?;
    Ok(Reply::new(renewed.renewed).with_cookie(secret_cookie(
        TICKET_COOKIE,
        &renewed.ticket.secret,
        renewed.ticket.max_age_s,
        SameSite::Strict,
    )))
}
