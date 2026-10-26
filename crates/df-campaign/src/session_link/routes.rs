//! Session ids locate a room; S1 credentials and the frozen Start roster authorize entry.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Request, State},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use df_types::SessionId;
use futures::future::BoxFuture;
use http::{HeaderValue, StatusCode, header};

use super::{Audience, Placement, RuntimeLauncher, SessionLauncher, Surface};
use crate::{
    audit::AuditLine,
    errors::{Failure, StoreError},
    records::{CampaignStatus, MemberStatus},
    routes::{AuthClass, DEVICE_COOKIE, Guard, GuardRefusal, TICKET_COOKIE, cookie_value},
    service::{CampaignService, Ports},
    store::{CampaignStore, SessionDirectory, SessionPhase},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TunnelAuthorization {
    Authorized,
    Revoked,
    Unavailable,
}

#[derive(Clone)]
pub(super) struct TunnelRevalidator {
    check: Arc<dyn Fn() -> BoxFuture<'static, TunnelAuthorization> + Send + Sync>,
}

impl TunnelRevalidator {
    pub(super) fn new(
        check: impl Fn() -> BoxFuture<'static, TunnelAuthorization> + Send + Sync + 'static,
    ) -> Self {
        Self {
            check: Arc::new(check),
        }
    }

    pub(super) fn check(&self) -> BoxFuture<'static, TunnelAuthorization> {
        (self.check)()
    }
}

pub struct App<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher> {
    pub service: CampaignService<P>,
    pub guard: Arc<G>,
    pub launcher: SessionLauncher<D, L>,
    pub origin: Arc<str>,
}

impl<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher> Clone for App<P, G, D, L> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            guard: Arc::clone(&self.guard),
            launcher: self.launcher.clone(),
            origin: Arc::clone(&self.origin),
        }
    }
}

/// Merge at the origin root; HOSTEDSTACK must use this same launcher as `Ports::Sessions`.
pub fn router<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    app: App<P, G, D, L>,
) -> Router {
    Router::new()
        .route("/sessions/{session}/{surface}", get(entry::<P, G, D, L>))
        .with_state(app)
}

async fn entry<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    State(app): State<App<P, G, D, L>>,
    Path((session, surface)): Path<(String, String)>,
    request: Request,
) -> Response {
    match tokio::time::timeout(
        app.launcher.inner.deadline,
        serve_entry(&app, &session, &surface, request),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => {
            AuditLine::new("SESSION", "entry")
                .reason("deadline")
                .emit("session.entry");
            waiting()
        }
    }
}

async fn serve_entry<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    app: &App<P, G, D, L>,
    session: &str,
    surface: &str,
    request: Request,
) -> Response {
    let surface = match surface {
        "play" => Surface::Play,
        "display" => Surface::Display,
        "tunnel" => Surface::Tunnel,
        _ => return concealed(),
    };
    let Some(session) =
        crate::secrets::unhex(session).and_then(|bytes| SessionId::from_bytes(&bytes).ok())
    else {
        return concealed();
    };
    let Some(placement) = app.launcher.placement().await else {
        return concealed();
    };
    if placement.session != session {
        return concealed();
    }
    let (parts, _) = request.into_parts();
    if surface == Surface::Tunnel
        && parts
            .headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            != Some(app.origin.as_ref())
    {
        return concealed();
    }
    let audience = match authorize(app, &placement, surface, &parts).await {
        Ok(Some(audience)) => audience,
        Ok(None) => return concealed(),
        Err(_) => {
            record_entry(&placement, "authority_unavailable");
            return waiting();
        }
    };
    let usable = match app.launcher.session(session).await {
        Ok(Some(facts)) => {
            facts.campaign == placement.campaign
                && matches!(
                    facts.phase,
                    SessionPhase::PreparingFundingOpen | SessionPhase::Playing
                )
        }
        Ok(None) => false,
        Err(_) => {
            record_entry(&placement, "directory_unavailable");
            return waiting();
        }
    };
    if !usable {
        return concealed();
    }
    let revalidator = if surface == Surface::Tunnel {
        match tunnel_revalidator(app, &placement, audience, &parts) {
            Some(revalidator) => Some(revalidator),
            None => return concealed(),
        }
    } else {
        None
    };
    serve_runtime(app, &placement, audience, surface, revalidator, parts).await
}

async fn serve_runtime<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    app: &App<P, G, D, L>,
    placement: &Placement,
    audience: Audience,
    surface: Surface,
    revalidator: Option<TunnelRevalidator>,
    mut parts: http::request::Parts,
) -> Response {
    let room = match app.launcher.allocate(placement).await {
        Ok(room) => room,
        Err(_) => return waiting(),
    };
    // The runtime receives the verified typed audience, never account/device credentials or
    // a caller-supplied launcher header. Keep only the headers needed for a WebSocket upgrade.
    let original_headers = std::mem::take(&mut parts.headers);
    for name in [
        "origin",
        "upgrade",
        "connection",
        "sec-websocket-key",
        "sec-websocket-version",
        "sec-websocket-protocol",
        "sec-websocket-extensions",
    ] {
        if let Some(value) = original_headers.get(name) {
            parts
                .headers
                .insert(http::header::HeaderName::from_static(name), value.clone());
        }
    }
    if let Some(revalidator) = revalidator {
        parts.extensions.insert(revalidator);
    }
    let request = Request::from_parts(parts, axum::body::Body::empty());
    match tokio::time::timeout(
        app.launcher.inner.deadline,
        app.launcher
            .inner
            .runtime
            .serve(placement, &room, audience, surface, request),
    )
    .await
    {
        Ok(Ok(response)) => {
            record_entry(placement, "served");
            private(response)
        }
        Ok(Err(_)) | Err(_) => {
            record_entry(placement, "runtime_unavailable");
            waiting()
        }
    }
}

fn tunnel_revalidator<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    app: &App<P, G, D, L>,
    placement: &Placement,
    audience: Audience,
    parts: &http::request::Parts,
) -> Option<TunnelRevalidator> {
    match audience {
        Audience::Player(member) => {
            let account = placement
                .roster
                .iter()
                .find(|seat| seat.member == member)?
                .account;
            let service = app.service.clone();
            let launcher = app.launcher.clone();
            let guard = Arc::clone(&app.guard);
            let headers = parts.headers.clone();
            let campaign = placement.campaign;
            let session = placement.session;
            Some(TunnelRevalidator::new(move || {
                let service = service.clone();
                let launcher = launcher.clone();
                let guard = Arc::clone(&guard);
                let headers = headers.clone();
                Box::pin(async move {
                    let mut request = Request::new(axum::body::Body::empty());
                    *request.headers_mut() = headers;
                    let (parts, _) = request.into_parts();
                    match guard.caller(&parts, AuthClass::Session).await {
                        Ok(caller) if caller.account == Some(account) => {}
                        Ok(_) | Err(GuardRefusal::Unauthenticated | GuardRefusal::CsrfRefused) => {
                            return TunnelAuthorization::Revoked;
                        }
                        Err(GuardRefusal::Unavailable) => {
                            return TunnelAuthorization::Unavailable;
                        }
                    }
                    let member_snapshot = match service.member_snapshot(account, campaign).await {
                        Ok(Some(snapshot)) => snapshot,
                        Ok(None) => return TunnelAuthorization::Revoked,
                        Err(Failure::Refused(_)) => return TunnelAuthorization::Revoked,
                        Err(Failure::Store(_)) => return TunnelAuthorization::Unavailable,
                    };
                    if member_snapshot.member != member
                        || member_snapshot.account != account
                        || !member_snapshot.authorized
                        || !member_snapshot.holds_seat
                        || member_snapshot.status != MemberStatus::Active
                        || member_snapshot.campaign_status != CampaignStatus::Active
                    {
                        return TunnelAuthorization::Revoked;
                    }
                    match launcher.session(session).await {
                        Ok(Some(facts))
                            if facts.campaign == campaign
                                && matches!(
                                    facts.phase,
                                    SessionPhase::PreparingFundingOpen | SessionPhase::Playing
                                ) =>
                        {
                            TunnelAuthorization::Authorized
                        }
                        Ok(_) => TunnelAuthorization::Revoked,
                        Err(_) => TunnelAuthorization::Unavailable,
                    }
                })
            }))
        }
        Audience::Display => {
            let ticket = cookie_value(&parts.headers, TICKET_COOKIE)?.to_owned();
            let device = cookie_value(&parts.headers, DEVICE_COOKIE)?.to_owned();
            let service = app.service.clone();
            let launcher = app.launcher.clone();
            let campaign = placement.campaign;
            let session = placement.session;
            Some(TunnelRevalidator::new(move || {
                let service = service.clone();
                let launcher = launcher.clone();
                let ticket = ticket.clone();
                let device = device.clone();
                Box::pin(async move {
                    match service
                        .verify_display_ticket(&ticket, &device, session)
                        .await
                    {
                        Ok(_) => {}
                        Err(Failure::Refused(_)) => return TunnelAuthorization::Revoked,
                        Err(Failure::Store(_)) => return TunnelAuthorization::Unavailable,
                    }
                    match launcher.session(session).await {
                        Ok(Some(facts))
                            if facts.campaign == campaign
                                && matches!(
                                    facts.phase,
                                    SessionPhase::PreparingFundingOpen | SessionPhase::Playing
                                ) =>
                        {
                            TunnelAuthorization::Authorized
                        }
                        Ok(_) => TunnelAuthorization::Revoked,
                        Err(_) => TunnelAuthorization::Unavailable,
                    }
                })
            }))
        }
    }
}

async fn authorize<P: Ports, G: Guard, D: SessionDirectory, L: RuntimeLauncher>(
    app: &App<P, G, D, L>,
    placement: &Placement,
    surface: Surface,
    parts: &http::request::Parts,
) -> Result<Option<Audience>, StoreError> {
    if surface != Surface::Play {
        let device = cookie_value(&parts.headers, DEVICE_COOKIE);
        let ticket = cookie_value(&parts.headers, TICKET_COOKIE);
        if let (Some(device), Some(ticket)) = (device, ticket) {
            match app
                .service
                .verify_display_ticket(&ticket, &device, placement.session)
                .await
            {
                Ok(_) => return Ok(Some(Audience::Display)),
                Err(Failure::Store(error)) => return Err(error),
                Err(Failure::Refused(_)) => {}
            }
        }
        if surface == Surface::Display {
            return Ok(None);
        }
    }
    let caller = match app.guard.caller(parts, AuthClass::Session).await {
        Ok(caller) => caller,
        Err(GuardRefusal::Unavailable) => return Err(StoreError::Unavailable),
        Err(GuardRefusal::Unauthenticated | GuardRefusal::CsrfRefused) => return Ok(None),
    };
    let Some(account) = caller.account else {
        return Ok(None);
    };
    let Some(member) = app
        .service
        .store()
        .member_snapshot(account, placement.campaign)
        .await?
    else {
        return Ok(None);
    };
    if !member.authorized
        || !member.holds_seat
        || member.status != MemberStatus::Active
        || member.campaign_status != CampaignStatus::Active
    {
        return Ok(None);
    }
    Ok(placement
        .roster
        .iter()
        .find(|seat| seat.account == account && seat.member == member.member)
        .map(|seat| Audience::Player(seat.member)))
}

fn record_entry(placement: &Placement, reason: &'static str) {
    AuditLine::new("SESSION", "entry")
        .campaign(placement.campaign)
        .session(placement.session)
        .reason(reason)
        .emit("session.entry");
}

fn concealed() -> Response {
    private(StatusCode::NOT_FOUND.into_response())
}

fn waiting() -> Response {
    private((StatusCode::SERVICE_UNAVAILABLE, [(header::RETRY_AFTER, "3")], Html(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>DungeonFlux - getting ready</title><main><h1>The table is getting ready</h1><p>Your session has started. The game is not ready yet.</p><p><a href=\"\">Retry</a></p></main></html>"
    )).into_response())
}

fn private(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
