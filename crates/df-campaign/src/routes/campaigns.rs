//! Handlers for campaigns and their members: creation, the caller's own membership, archive and
//! delete, leaving, removal, co-hosts, seats, roles and transfers.

use df_campaign_wire as wire;
use http::StatusCode;

use super::{
    guard::Guard,
    request::{
        Ctx, co_host_body, delegations_of, hex_of, join_role, not_found, operation_of, role_name,
    },
    responses::{Handled, failure_response, ok, refused},
};
use crate::{
    records::{MemberStatus, OldHostBecomes, RoomCodes, TransferKind},
    service::{CreateCampaign, Ports},
    store::NewProposal,
};

pub(super) async fn create_campaign<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::CampaignCreateBody = ctx.json()?;
    let account = ctx.account()?;
    let service = ctx.service();
    let created = service
        .create_campaign(
            account,
            CreateCampaign {
                name: body.name,
                seat_count: body.seat_count,
                content_rating: body.content_rating,
                room_codes: if body.room_codes {
                    RoomCodes::WithApproval
                } else {
                    RoomCodes::Off
                },
                default_co_host_delegations: delegations_of(&body.default_co_host),
                operation: operation_of(&body.operation)?,
            },
        )
        .await
        .map_err(failure_response)?;
    let snapshot = service
        .member_snapshot(account, created.id)
        .await
        .map_err(failure_response)?
        .ok_or_else(not_found)?;
    ok(&wire::CampaignCreated {
        campaign_id: hex_of(created.id.as_bytes()),
        member_id: hex_of(snapshot.member.as_bytes()),
    })
}

/// A former member (left or removed) gets the same answer as a stranger: nothing about the
/// campaign's activity.
pub(super) async fn my_membership<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let snapshot = ctx
        .service()
        .member_snapshot(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?
        .filter(|snapshot| snapshot.status == MemberStatus::Active)
        .ok_or_else(not_found)?;
    ok(&wire::MeResponse {
        member_id: hex_of(snapshot.member.as_bytes()),
        role: role_name(snapshot.role),
        holds_seat: snapshot.holds_seat,
        delegations: co_host_body(snapshot.delegations),
        authorized: snapshot.authorized,
        member_revision: snapshot.member_revision,
        seat_count: snapshot.seat_count,
        event_seq: snapshot.event_seq,
    })
}

pub(super) async fn archive<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::TypedNameBody = ctx.json()?;
    ctx.service()
        .archive(ctx.account()?, ctx.campaign()?, body.typed_name)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn unarchive<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .unarchive(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn request_delete<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::TypedNameBody = ctx.json()?;
    ctx.service()
        .request_delete(ctx.account()?, ctx.campaign()?, body.typed_name)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn remove_member<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .remove_member(ctx.account()?, ctx.campaign()?, ctx.member()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn allow_rejoin<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .allow_rejoin(ctx.account()?, ctx.campaign()?, ctx.member()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn set_co_host<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::CoHostBody = ctx.json()?;
    ctx.service()
        .set_co_host(
            ctx.account()?,
            ctx.campaign()?,
            ctx.member()?,
            delegations_of(&body),
        )
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn demote_co_host<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .demote_co_host(ctx.account()?, ctx.campaign()?, ctx.member()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn leave<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    ctx.service()
        .leave(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn set_seat<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::SeatBody = ctx.json()?;
    ctx.service()
        .set_seat(ctx.account()?, ctx.campaign()?, body.seated)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn set_player_role<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::RoleBody = ctx.json()?;
    ctx.service()
        .set_player_role(
            ctx.account()?,
            ctx.campaign()?,
            ctx.member()?,
            join_role(body.role),
        )
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn propose_transfer<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::TransferProposeBody = ctx.json()?;
    let to_member = df_types::MemberId::from_hex(&body.to_member)
        .map_err(|_| refused(StatusCode::BAD_REQUEST, wire::Reason::InvalidInput))?;
    let kind = match body.kind {
        wire::TransferKindName::Host => TransferKind::Host,
        wire::TransferKindName::Ownership => TransferKind::Ownership,
    };
    let old_host_becomes = body.old_host_becomes.map(|value| match value {
        wire::OldHostBecomesName::Player => OldHostBecomes::Player,
        wire::OldHostBecomesName::CoHost => OldHostBecomes::CoHost,
    });
    let proposal = ctx
        .service()
        .propose_transfer(
            ctx.account()?,
            ctx.campaign()?,
            NewProposal {
                kind,
                to_member,
                old_host_becomes,
            },
        )
        .await
        .map_err(failure_response)?;
    ok(&wire::TransferProposed {
        proposal_id: proposal.text(),
    })
}

pub(super) async fn accept_transfer<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let proposal = proposal_id(ctx)?;
    ctx.service()
        .accept_transfer(ctx.account()?, ctx.campaign()?, proposal)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

pub(super) async fn cancel_transfer<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let proposal = proposal_id(ctx)?;
    ctx.service()
        .cancel_transfer(ctx.account()?, ctx.campaign()?, proposal)
        .await
        .map_err(failure_response)?;
    ok(&wire::Done {})
}

fn proposal_id<P: Ports, G: Guard>(
    ctx: &Ctx<'_, P, G>,
) -> Result<crate::ids::ProposalId, super::responses::Failed> {
    ctx.param("proposal")
        .and_then(crate::ids::ProposalId::parse)
        .ok_or_else(not_found)
}
