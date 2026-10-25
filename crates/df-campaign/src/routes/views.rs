//! Handlers for the reads the screens are built from: the dashboard, a campaign's overview, its
//! settings and its sessions tab. They change nothing.

use df_campaign_wire as wire;

use super::{
    drafts::{funding_name, picture_name, spend_name, voice_name},
    guard::Guard,
    request::{Ctx, co_host_body, hex_of, role_name},
    responses::{Handled, failure_response, ok},
};
use crate::{
    records::{CampaignStatus, RoomCodes},
    service::{Overview, Ports, SessionReading, Tab},
    store::SessionPhase,
};

pub(super) fn phase_name(phase: SessionPhase) -> wire::SessionPhaseName {
    match phase {
        SessionPhase::Lobby => wire::SessionPhaseName::Lobby,
        SessionPhase::PreparingFundingClosed | SessionPhase::PreparingFundingOpen => {
            wire::SessionPhaseName::Preparing
        }
        SessionPhase::Playing => wire::SessionPhaseName::Playing,
        SessionPhase::Ended => wire::SessionPhaseName::Ended,
    }
}

fn reading_view(reading: SessionReading) -> wire::SessionReadingView {
    match reading {
        SessionReading::Live(live) => wire::SessionReadingView::Live(wire::LiveSessionView {
            session_id: hex_of(live.session.as_bytes()),
            phase: phase_name(live.phase),
        }),
        SessionReading::None => wire::SessionReadingView::None,
        SessionReading::Unknown => wire::SessionReadingView::Unknown,
        SessionReading::NotChecked => wire::SessionReadingView::NotChecked,
    }
}

fn status_name(status: CampaignStatus) -> wire::CampaignStatusName {
    match status {
        CampaignStatus::Archived => wire::CampaignStatusName::Archived,
        // Draft and deleting campaigns are never listed; the store reads only the other two.
        CampaignStatus::Active | CampaignStatus::Draft | CampaignStatus::Deleting => {
            wire::CampaignStatusName::Active
        }
    }
}

fn tab_name(tab: Tab) -> wire::TabName {
    match tab {
        Tab::Overview => wire::TabName::Overview,
        Tab::Sessions => wire::TabName::Sessions,
        Tab::Characters => wire::TabName::Characters,
        Tab::Assets => wire::TabName::Assets,
        Tab::Funding => wire::TabName::Funding,
        Tab::Settings => wire::TabName::Settings,
    }
}

pub(super) async fn dashboard<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let dashboard = ctx
        .service()
        .dashboard(ctx.account()?)
        .await
        .map_err(failure_response)?;
    let view = wire::DashboardView {
        empty: dashboard.is_empty(),
        campaigns: dashboard
            .campaigns
            .into_iter()
            .map(|entry| wire::DashboardCampaign {
                campaign_id: hex_of(entry.row.id.as_bytes()),
                name: entry.row.name,
                status: status_name(entry.row.status),
                content_rating: entry.row.content_rating,
                role: role_name(entry.row.role),
                is_owner: entry.row.is_owner,
                seat_count: entry.row.seat_count,
                seats_in_use: entry.row.seats_in_use,
                planned_session_at_ms: entry.row.planned_session_at_ms,
                scheduled_delete_at_ms: entry.row.scheduled_delete_at_ms,
                session: reading_view(entry.session),
            })
            .collect(),
        drafts: dashboard
            .drafts
            .into_iter()
            .map(|draft| wire::DashboardDraft {
                draft_id: hex_of(draft.id.as_bytes()),
                name: draft.name,
                step: draft.step,
                expires_at_ms: draft.expires_at_ms,
            })
            .collect(),
    };
    ok(&view)
}

fn overview_view(overview: Overview, maximum_contributors: u32) -> wire::OverviewView {
    let me = &overview.me;
    wire::OverviewView {
        campaign_id: hex_of(overview.campaign.id.as_bytes()),
        name: overview.campaign.name.clone(),
        status: status_name(overview.campaign.status),
        content_rating: overview.campaign.content_rating,
        seat_count: overview.campaign.seat_count,
        seats_in_use: overview.seats_in_use,
        contributors_in_use: overview.contributors_in_use,
        maximum_contributors,
        planned_session_at_ms: overview.campaign.planned_session_at_ms,
        scheduled_delete_at_ms: overview.campaign.scheduled_delete_at_ms,
        room_codes: overview.campaign.room_codes == RoomCodes::WithApproval,
        members: overview
            .roster
            .iter()
            .map(|entry| wire::RosterEntryView {
                member_id: hex_of(entry.member.member.as_bytes()),
                display_name: entry.display_name.clone(),
                role: role_name(entry.member.role),
                holds_seat: entry.member.holds_seat,
                delegations: co_host_body(entry.member.delegations),
                is_you: entry.member.member == me.member,
            })
            .collect(),
        you: wire::MeResponse {
            member_id: hex_of(me.member.as_bytes()),
            role: role_name(me.role),
            holds_seat: me.holds_seat,
            delegations: co_host_body(me.delegations),
            authorized: true,
            member_revision: me.revision,
            seat_count: overview.campaign.seat_count,
            event_seq: overview.campaign.event_seq,
        },
        is_owner: overview.is_owner,
        pending_requests: overview.pending_requests,
        session: reading_view(overview.session),
        tabs: overview.tabs.iter().copied().map(tab_name).collect(),
        can_invite: overview.can_invite,
        can_start: overview.can_start,
        can_edit_settings: overview.can_edit_settings,
    }
}

pub(super) async fn overview<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let service = ctx.service();
    let overview = service
        .overview(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&overview_view(
        overview,
        service.tunables().maximum_contributors,
    ))
}

pub(super) async fn settings<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let overview = ctx
        .service()
        .overview(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    let settings = &overview.settings;
    let campaign = &overview.campaign;
    ok(&wire::SettingsView {
        campaign_id: hex_of(campaign.id.as_bytes()),
        name: campaign.name.clone(),
        adventure_kind: settings.adventure_kind.clone(),
        content_rating: campaign.content_rating,
        seat_count: campaign.seat_count,
        voice_default: voice_name(settings.voice_default),
        picture_default: picture_name(settings.picture_default),
        spend_preset: spend_name(settings.spend_preset),
        custom_hourly_micro_dollars: settings.custom_hourly_micro_dollars,
        session_ceiling_micro_dollars: settings.session_ceiling_micro_dollars,
        default_co_host: co_host_body(campaign.default_co_host_delegations),
        room_codes: campaign.room_codes == RoomCodes::WithApproval,
        funding_model: funding_name(settings.funding_model),
        display_summary_enabled: campaign.display_summary_enabled,
        editable: overview.can_edit_settings,
    })
}

pub(super) async fn sessions<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let reading = ctx
        .service()
        .sessions(ctx.account()?, ctx.campaign()?)
        .await
        .map_err(failure_response)?;
    ok(&wire::SessionsView {
        session: reading_view(reading),
    })
}
