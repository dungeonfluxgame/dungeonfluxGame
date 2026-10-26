//! The JSON route module that B mounts under `/app/api/` (`planning/campaigns-and-table-research.md`
//! section 7). It serves JSON only, never HTML. Every route is a row of [`ROUTES`] and the router
//! is built from that table, so the table the tests walk is the router that is served.
//!
//! Identity comes from B's [`Guard`] (session, CSRF, trusted-proxy client address); this module
//! never reads a display cookie as an identity on a session route. The display device routes
//! check the cookie, `Origin`, `Sec-Fetch-Site`, `X-DF-Device` and the content type themselves
//! and use no CSRF token (8.5).

mod campaigns;
mod cookies;
mod drafts;
mod guard;
mod joining;
mod lobby;
mod request;
mod responses;
mod table;
mod views;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::{
    Router,
    extract::{RawPathParams, Request, State},
    response::Response,
    routing::{MethodFilter, MethodRouter, on},
};
use df_campaign_wire as wire;
use http::StatusCode;

use self::{
    request::Ctx,
    responses::{Handled, guard_response, refused},
};
use crate::service::{CampaignService, Ports};

pub use cookies::{
    DEVICE_COOKIE, DEVICE_HEADER, DisplayRequestRefusal, TICKET_COOKIE, check_display_request,
    clear_cookie, cookie_value, set_cookie,
};
pub use guard::{Caller, Guard, GuardRefusal};
pub use responses::{ends_display_pairing, failure_reason, refusal_reason};
pub use table::{AuthClass, Gate, ROUTES, RouteId, RouteSpec, Verb};

const MAXIMUM_BODY_BYTES: usize = 16 * 1024;

pub struct App<P: Ports, G: Guard> {
    pub service: CampaignService<P>,
    pub guard: Arc<G>,
    /// The configured origin (`https://host`), for the display request checks.
    pub origin: Arc<str>,
}

impl<P: Ports, G: Guard> Clone for App<P, G> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            guard: Arc::clone(&self.guard),
            origin: Arc::clone(&self.origin),
        }
    }
}

/// The router B nests under `/app/api/`. Built from [`ROUTES`].
pub fn router<P: Ports, G: Guard>(app: App<P, G>) -> Router {
    let mut router: Router<App<P, G>> = Router::new();
    for route in ROUTES {
        router = router.route(route.path, mount::<P, G>(route));
    }
    router.with_state(app)
}

fn mount<P: Ports, G: Guard>(route: &'static RouteSpec) -> MethodRouter<App<P, G>> {
    let filter = match route.verb {
        Verb::Get => MethodFilter::GET,
        Verb::Post => MethodFilter::POST,
        Verb::Put => MethodFilter::PUT,
    };
    on(
        filter,
        move |State(app): State<App<P, G>>, params: RawPathParams, request: Request| {
            let params: Vec<(String, String)> = params
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
            async move { dispatch(app, route, params, request).await }
        },
    )
}

async fn dispatch<P: Ports, G: Guard>(
    app: App<P, G>,
    route: &'static RouteSpec,
    params: Vec<(String, String)>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let caller = match app.guard.caller(&parts, route.auth).await {
        Ok(caller) => caller,
        Err(refusal) => return guard_response(refusal),
    };
    let bytes = match axum::body::to_bytes(body, MAXIMUM_BODY_BYTES).await {
        Ok(bytes) => bytes.to_vec(),
        Err(_) => return *refused(StatusCode::BAD_REQUEST, wire::Reason::InvalidInput),
    };
    let ctx = Ctx {
        app: &app,
        params,
        caller,
        body: bytes,
    };
    match handle(route.id, &ctx).await {
        Ok(response) => response,
        Err(response) => *response,
    }
}

/// One line per route: the handler that serves it.
async fn handle<P: Ports, G: Guard>(id: RouteId, ctx: &Ctx<'_, P, G>) -> Handled {
    match id {
        RouteId::CreateCampaign => campaigns::create_campaign(ctx).await,
        RouteId::Dashboard => views::dashboard(ctx).await,
        RouteId::Overview => views::overview(ctx).await,
        RouteId::CampaignSettings => views::settings(ctx).await,
        RouteId::CampaignSessions => views::sessions(ctx).await,
        RouteId::OpenLobby => lobby::open(ctx).await,
        RouteId::LobbyView => lobby::view(ctx).await,
        RouteId::LobbyStart => lobby::start(ctx).await,
        RouteId::StartDraft => drafts::start(ctx).await,
        RouteId::GetDraft => drafts::get(ctx).await,
        RouteId::SaveDraftStep => drafts::save_step(ctx).await,
        RouteId::FinishDraft => drafts::finish(ctx).await,
        RouteId::MyMembership => campaigns::my_membership(ctx).await,
        RouteId::Archive => campaigns::archive(ctx).await,
        RouteId::Unarchive => campaigns::unarchive(ctx).await,
        RouteId::RequestDelete => campaigns::request_delete(ctx).await,
        RouteId::RemoveMember => campaigns::remove_member(ctx).await,
        RouteId::AllowRejoin => campaigns::allow_rejoin(ctx).await,
        RouteId::SetCoHost => campaigns::set_co_host(ctx).await,
        RouteId::DemoteCoHost => campaigns::demote_co_host(ctx).await,
        RouteId::Leave => campaigns::leave(ctx).await,
        RouteId::SetSeat => campaigns::set_seat(ctx).await,
        RouteId::SetPlayerRole => campaigns::set_player_role(ctx).await,
        RouteId::ProposeTransfer => campaigns::propose_transfer(ctx).await,
        RouteId::AcceptTransfer => campaigns::accept_transfer(ctx).await,
        RouteId::CancelTransfer => campaigns::cancel_transfer(ctx).await,
        RouteId::CreateInvitation => joining::create_invitation(ctx).await,
        RouteId::RevokeInvitation => joining::revoke_invitation(ctx).await,
        RouteId::RotateCode => joining::rotate_code(ctx).await,
        RouteId::JoinPreview => joining::join_preview(ctx).await,
        RouteId::JoinAccept => joining::join_accept(ctx).await,
        RouteId::CodeLookup => joining::code_lookup(ctx).await,
        RouteId::CodeRequest => joining::code_request(ctx).await,
        RouteId::ApproveRequest => joining::approve_request(ctx).await,
        RouteId::DenyRequest => joining::deny_request(ctx).await,
    }
}
