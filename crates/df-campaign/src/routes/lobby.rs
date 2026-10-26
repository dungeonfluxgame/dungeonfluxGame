//! Handlers for the lobby (8.4). The session id travels with its campaign in the path, as for the
//! display routes: a session carries no foreign key, so the service checks the pair.

use df_campaign_wire as wire;
use http::StatusCode;

use super::{
    guard::Guard,
    request::{Ctx, hex_of, role_name},
    responses::{Handled, failure_response, json_response, ok},
    views::phase_name,
};
use crate::service::{LobbyView, Ports, StartBlocker, StartOutcome};

fn blocker_name(blocker: StartBlocker) -> wire::StartBlockerName {
    match blocker {
        StartBlocker::CampaignArchived => wire::StartBlockerName::CampaignArchived,
        StartBlocker::SessionNotInLobby => wire::StartBlockerName::SessionNotInLobby,
        StartBlocker::NoSeatedMembers => wire::StartBlockerName::NoSeatedMembers,
        StartBlocker::FundingNotReady => wire::StartBlockerName::FundingNotReady,
        StartBlocker::RosterInconsistent => wire::StartBlockerName::RosterInconsistent,
    }
}

fn lobby_view(view: LobbyView) -> wire::LobbyView {
    wire::LobbyView {
        campaign_id: hex_of(view.campaign.id.as_bytes()),
        session_id: hex_of(view.session.as_bytes()),
        phase: phase_name(view.phase),
        seat_count: view.campaign.seat_count,
        seats: view
            .seats
            .iter()
            .map(|seat| wire::LobbySeatView {
                member_id: hex_of(seat.member.member.as_bytes()),
                display_name: seat.display_name.clone(),
                role: role_name(seat.member.role),
                can_play: seat.authorized,
                is_you: seat.member.member == view.you,
            })
            .collect(),
        blockers: view.blockers.iter().copied().map(blocker_name).collect(),
        funding: view.funding.map(|readiness| wire::FundingNotice {
            ready: readiness.ready,
            waiting_for_sponsor: readiness.waiting_for_sponsor,
            no_charge_until_start: readiness.no_charge_until_start,
        }),
        can_start: view.can_start,
    }
}

pub(super) async fn open<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let (session, phase) = ctx
        .service()
        .open_lobby(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::LobbyOpened {
        session_id: hex_of(session.as_bytes()),
        phase: phase_name(phase),
    })
}

pub(super) async fn view<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let lobby = ctx
        .service()
        .lobby_view(ctx.account()?, ctx.campaign()?, ctx.session()?)
        .await
        .map_err(failure_response)?;
    ok(&lobby_view(lobby))
}

pub(super) async fn start<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let outcome = ctx
        .service()
        .start_session(ctx.account()?, ctx.campaign()?, ctx.session()?)
        .await
        .map_err(failure_response)?;
    match outcome {
        StartOutcome::Started { roster } => ok(&wire::StartedBody {
            seated: u32::try_from(roster.len()).unwrap_or(u32::MAX),
        }),
        StartOutcome::Blocked(blockers) => Ok(json_response(
            StatusCode::CONFLICT,
            &wire::StartBlockedBody {
                reason: wire::Reason::StartBlocked,
                blockers: blockers.into_iter().map(blocker_name).collect(),
            },
        )),
    }
}
