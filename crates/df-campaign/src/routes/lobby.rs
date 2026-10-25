//! Handlers for the lobby (8.4). The session id travels with its campaign in the path, as for the
//! display routes: a session carries no foreign key, so the service checks the pair.

use df_campaign_wire as wire;
use http::StatusCode;

use super::{
    guard::Guard,
    request::{Ctx, hex_of, invalid, role_name},
    responses::{Handled, failure_response, json_response, ok},
    views::phase_name,
};
use crate::{
    lobby::{CharacterChoice, StartBlocker},
    lobby_state::{LobbyMemberRow, MicrophonePreference, MicrophoneReport},
    service::{LobbyView, Ports, StartOutcome},
};

fn character_name(choice: CharacterChoice) -> wire::CharacterChoiceName {
    match choice {
        CharacterChoice::Undecided => wire::CharacterChoiceName::Undecided,
        CharacterChoice::Template => wire::CharacterChoiceName::Template,
        CharacterChoice::Existing => wire::CharacterChoiceName::Existing,
    }
}

fn preference_name(value: MicrophonePreference) -> wire::MicrophonePreferenceName {
    match value {
        MicrophonePreference::Ask => wire::MicrophonePreferenceName::Ask,
        MicrophonePreference::TypedOnly => wire::MicrophonePreferenceName::TypedOnly,
    }
}

fn report_name(value: MicrophoneReport) -> wire::MicrophoneReportName {
    match value {
        MicrophoneReport::Unknown => wire::MicrophoneReportName::Unknown,
        MicrophoneReport::Granted => wire::MicrophoneReportName::Granted,
        MicrophoneReport::Declined => wire::MicrophoneReportName::Declined,
        MicrophoneReport::Unsupported => wire::MicrophoneReportName::Unsupported,
    }
}

fn blocker_name(blocker: StartBlocker) -> wire::StartBlockerName {
    match blocker {
        StartBlocker::CampaignArchived => wire::StartBlockerName::CampaignArchived,
        StartBlocker::SessionNotInLobby => wire::StartBlockerName::SessionNotInLobby,
        StartBlocker::NoSeatedMembers => wire::StartBlockerName::NoSeatedMembers,
        StartBlocker::CharacterNotChosen => wire::StartBlockerName::CharacterNotChosen,
        StartBlocker::MemberNotReady => wire::StartBlockerName::MemberNotReady,
        StartBlocker::DisplayRequiredButUnpaired => {
            wire::StartBlockerName::DisplayRequiredButUnpaired
        }
        StartBlocker::FundingNotReady => wire::StartBlockerName::FundingNotReady,
        StartBlocker::RosterInconsistent => wire::StartBlockerName::RosterInconsistent,
    }
}

fn changed(row: &LobbyMemberRow) -> wire::LobbyMemberChanged {
    wire::LobbyMemberChanged {
        character: character_name(row.character),
        ready: row.ready,
        microphone_preference: preference_name(row.microphone_preference),
        microphone_report: report_name(row.microphone_report),
        revision: row.revision,
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
                character: character_name(seat.character()),
                character_ref: seat
                    .lobby
                    .as_ref()
                    .and_then(|row| row.character_ref)
                    .map(|bytes| hex_of(&bytes)),
                ready: seat.ready(),
                microphone_preference: preference_name(
                    seat.lobby
                        .as_ref()
                        .map_or(MicrophonePreference::Ask, |row| row.microphone_preference),
                ),
                microphone_report: report_name(
                    seat.lobby
                        .as_ref()
                        .map_or(MicrophoneReport::Unknown, |row| row.microphone_report),
                ),
                is_you: seat.member.member == view.you,
            })
            .collect(),
        displays: view
            .displays
            .iter()
            .map(|display| wire::LobbyDisplayView {
                ticket_id: display.ticket.text(),
                device_label: display.label,
                expires_at_ms: display.expires_at_ms,
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

pub(super) async fn character<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::CharacterBody = ctx.json()?;
    let choice = match body.choice {
        wire::CharacterChoiceName::Undecided => CharacterChoice::Undecided,
        wire::CharacterChoiceName::Template => CharacterChoice::Template,
        wire::CharacterChoiceName::Existing => CharacterChoice::Existing,
    };
    let reference = match body.character_ref {
        Some(text) => Some(parse_reference(&text)?),
        None => None,
    };
    let row = ctx
        .service()
        .choose_character(
            ctx.account()?,
            ctx.campaign()?,
            ctx.session()?,
            choice,
            reference,
        )
        .await
        .map_err(failure_response)?;
    ok(&changed(&row))
}

/// 32 lowercase hex digits.
fn parse_reference(text: &str) -> Result<[u8; 16], super::responses::Failed> {
    crate::secrets::unhex(text)
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .ok_or_else(invalid)
}

pub(super) async fn ready<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    set_ready(ctx, true).await
}

pub(super) async fn unready<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    set_ready(ctx, false).await
}

async fn set_ready<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>, ready: bool) -> Handled {
    let row = ctx
        .service()
        .set_ready(ctx.account()?, ctx.campaign()?, ctx.session()?, ready)
        .await
        .map_err(failure_response)?;
    ok(&changed(&row))
}

pub(super) async fn microphone<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::MicrophoneBody = ctx.json()?;
    let preference = match body.preference {
        wire::MicrophonePreferenceName::Ask => MicrophonePreference::Ask,
        wire::MicrophonePreferenceName::TypedOnly => MicrophonePreference::TypedOnly,
    };
    let report = match body.report {
        wire::MicrophoneReportName::Unknown => MicrophoneReport::Unknown,
        wire::MicrophoneReportName::Granted => MicrophoneReport::Granted,
        wire::MicrophoneReportName::Declined => MicrophoneReport::Declined,
        wire::MicrophoneReportName::Unsupported => MicrophoneReport::Unsupported,
    };
    let row = ctx
        .service()
        .set_microphone(
            ctx.account()?,
            ctx.campaign()?,
            ctx.session()?,
            preference,
            report,
        )
        .await
        .map_err(failure_response)?;
    ok(&changed(&row))
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
