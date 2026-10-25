//! Handlers for invitations, room codes and join requests.

use df_campaign_wire as wire;

use super::{
    guard::Guard,
    request::{Ctx, invalid, join_role, not_found, operation_of},
    responses::{Failed, Handled, failure_response, ok},
};
use crate::{
    ids::{InvitationId, RequestId},
    service::Ports,
};

pub(super) async fn create_invitation<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::InvitationCreateBody = ctx.json()?;
    let (created, _) = ctx
        .service()
        .create_invitation(
            ctx.account()?,
            ctx.campaign()?,
            join_role(body.joins_as),
            body.lifetime_hours,
            body.max_uses,
        )
        .await
        .map_err(failure_response)?;
    ok(&created)
}

pub(super) async fn revoke_invitation<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let invitation = ctx
        .param("invitation")
        .and_then(InvitationId::parse)
        .ok_or_else(not_found)?;
    ctx.service()
        .revoke_invitation(ctx.account()?, ctx.campaign()?, invitation)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn rotate_code<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let (created, _) = ctx
        .service()
        .rotate_code(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&created)
}

pub(super) async fn join_preview<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::JoinTokenBody = ctx.json()?;
    let preview = ctx
        .service()
        .preview_token(ctx.caller.client_ip, &body.token)
        .await
        .map_err(failure_response)?;
    ok(&wire::JoinPreviewResponse { preview })
}

pub(super) async fn join_accept<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::JoinAcceptBody = ctx.json()?;
    // The pending-join cookie belongs to the shell (P and B); without a token there is nothing
    // to accept here.
    let token: String = body.token.ok_or_else(invalid)?;
    let accepted = ctx
        .service()
        .accept_token(ctx.account()?, &token, operation_of(&body.operation)?)
        .await
        .map_err(failure_response)?;
    ok(&wire::JoinAcceptResponse {
        campaign_id: super::request::hex_of(accepted.campaign.as_bytes()),
        member_id: super::request::hex_of(accepted.member.as_bytes()),
        rejoined: accepted.rejoined,
    })
}

pub(super) async fn code_lookup<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::CodeBody = ctx.json()?;
    let preview = ctx
        .service()
        .lookup_code(ctx.caller.client_ip, ctx.account()?, &body.code)
        .await
        .map_err(failure_response)?;
    ok(&wire::CodeLookupResponse { preview })
}

pub(super) async fn code_request<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::CodeBody = ctx.json()?;
    let request = ctx
        .service()
        .request_join(ctx.caller.client_ip, ctx.account()?, &body.code)
        .await
        .map_err(failure_response)?;
    ok(&wire::CodeRequestResponse {
        request_id: request.id.text(),
    })
}

fn request_id<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Result<RequestId, Failed> {
    ctx.param("request")
        .and_then(RequestId::parse)
        .ok_or_else(not_found)
}

pub(super) async fn approve_request<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::DecideRequestBody = ctx.json()?;
    ctx.service()
        .decide_request(
            ctx.account()?,
            ctx.campaign()?,
            request_id(ctx)?,
            Some(join_role(body.role)),
        )
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn deny_request<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .decide_request(ctx.account()?, ctx.campaign()?, request_id(ctx)?, None)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}
