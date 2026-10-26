//! A campaign's overview at `/app/campaigns/:id`: members, invitations, the room code, people
//! waiting to join, and the way into the lobby (plan 9.4, 9.6; S1 8.2, 8.3, 8.6).

use super::{actions, is_busy, panel, pill, row_button};
use crate::{
    copy, money,
    node::{Element, Node, el},
    route::Route,
    state::{CampaignScreen, Field, Form, Intent, Remote, RequestDecision},
    view::{
        ButtonKind, RadioChoice, button, checkbox, error_summary, fact, facts, failed, form, link,
        loading, page, page_wide, radio_group, status_line, submit_button,
    },
    wire_campaign::{
        CampaignOverview, FundingModel, InvitationKind, InvitationRow, JoinMode, JoinRequestRow,
        MemberRow, RoomCodeState,
    },
};

pub(in crate::view) fn overview(screen: &CampaignScreen, offset_east_minutes: i32) -> Node {
    let overview = match &screen.overview {
        Remote::Loading => {
            return page("Campaign", None)
                .child(loading("Opening your campaign"))
                .into();
        }
        Remote::Failed(error) => return page("Campaign", None).child(failed(*error)).into(),
        Remote::Ready(overview) => overview,
    };
    let is_busy = is_busy(screen.action, &screen.operation);
    let last_saved = overview.last_saved_at_ms.map_or_else(
        || "Not saved yet".to_owned(),
        |at_ms| money::date_time(at_ms, offset_east_minutes),
    );
    let mut sections = el("div").class("stack");
    sections = sections.child(play_panel(overview, is_busy));
    if overview.can_invite {
        sections = sections
            .child(requests_panel(
                &overview.join_requests,
                offset_east_minutes,
                is_busy,
            ))
            .child(members_panel(&overview.members))
            .child(invitations_panel(
                screen,
                overview,
                offset_east_minutes,
                is_busy,
            ))
            .child(room_code_panel(&overview.room_code, is_busy));
    } else {
        sections = sections.child(members_panel(&overview.members));
    }
    if let Some(settings) = screen.settings.as_ref()
        && screen.can_edit_settings
    {
        let error = screen.action.error();
        sections = sections.child(
            form(Form::CampaignSettings, "Campaign settings")
                .child(el("h2").text("Campaign settings"))
                .child(el("fieldset").class("choices").child(
                    el("legend").text("What co-hosts can do"),
                ).child(checkbox(
                    Field::CoHostsCanInvite,
                    "Co-hosts can invite people",
                    settings.draft.co_hosts_can_invite,
                    error,
                )).child(checkbox(
                    Field::CoHostsCanStart,
                    "Co-hosts can start sessions",
                    settings.draft.co_hosts_can_start,
                    error,
                )).child(checkbox(
                    Field::CoHostsCanExport,
                    "Co-hosts can export the campaign",
                    settings.draft.co_hosts_can_export,
                    error,
                )))
                .child(radio_group(
                    "Who pays for the table",
                    Field::FundingChoice,
                    [FundingModel::Sponsor, FundingModel::SharedPot]
                        .into_iter()
                        .map(|funding| RadioChoice {
                            field: Field::FundingChoice,
                            value: match funding {
                                FundingModel::Sponsor => "sponsor",
                                FundingModel::SharedPot => "shared",
                            },
                            title: match funding {
                                FundingModel::Sponsor => "I will sponsor this table",
                                FundingModel::SharedPot => "Shared pot",
                            }
                            .to_owned(),
                            detail: match funding {
                                FundingModel::Sponsor => {
                                    "Your credit pays for play, and players join without paying."
                                }
                                FundingModel::SharedPot => {
                                    "Players and contributors add their own credit to one pot for the table."
                                }
                            }
                            .to_owned(),
                            is_checked: settings.draft.funding == funding,
                        })
                        .collect(),
                    error,
                ))
                .child(radio_group(
                    "How people join",
                    Field::JoinModeChoice,
                    [JoinMode::LinkOnly, JoinMode::LinkAndRoomCode]
                        .into_iter()
                        .map(|mode| RadioChoice {
                            field: Field::JoinModeChoice,
                            value: match mode {
                                JoinMode::LinkOnly => "link",
                                JoinMode::LinkAndRoomCode => "code",
                            },
                            title: match mode {
                                JoinMode::LinkOnly => "Invitation link only",
                                JoinMode::LinkAndRoomCode => "Invitation link and room code",
                            }
                            .to_owned(),
                            detail: match mode {
                                JoinMode::LinkOnly => {
                                    "Only people you send an invitation link to can join."
                                }
                                JoinMode::LinkAndRoomCode => {
                                    "People can also ask to join by room code. You approve each request."
                                }
                            }
                            .to_owned(),
                            is_checked: settings.draft.join_mode == mode,
                        })
                        .collect(),
                    error,
                ))
                .child(submit_button(
                    "Save settings",
                    "Saving...",
                    screen.action.is_sending(),
                )),
        );
    }
    if let Some(error) = screen.settings_error {
        sections = sections.child(failed(error));
    }
    let mut shell = page_wide(&overview.name, None).child(error_summary(screen.action.error()));
    if let Remote::Failed(error) = &screen.operation {
        shell = shell.child(failed(*error));
    }
    shell
        .child(facts(vec![
            fact("Your role", copy::role_name(overview.my_role)),
            fact("Last saved", last_saved),
        ]))
        .child(sections.child(leave_panel(screen, &overview.name, is_busy)))
        .into()
}

fn play_panel(overview: &CampaignOverview, is_busy: bool) -> Element {
    let panel = panel("Play");
    match (&overview.live_session_id, overview.can_open_lobby) {
        (Some(session_id), _) => panel
            .child(el("p").text("A session is open for this campaign."))
            .child(actions(vec![
                link(
                    "See who has joined",
                    Route::Lobby {
                        session_id: session_id.clone(),
                    },
                    "btn btn-outline",
                ),
                link(
                    "Open the game",
                    Route::Play {
                        session_id: session_id.clone(),
                    },
                    "btn btn-gilt",
                ),
            ])),
        (None, true) => panel
            .child(el("p").text("Open the lobby when the table is ready to gather."))
            .child(actions(vec![button(
                "Open the lobby",
                Some(Intent::OpenLobby),
                ButtonKind::Primary,
                is_busy,
            )])),
        (None, false) => panel.child(
            el("p").text("The lobby is not open yet. The host opens it when the table is ready."),
        ),
    }
}

fn members_panel(members: &[MemberRow]) -> Element {
    let mut list = el("ul").class("rows");
    for member in members {
        let mut title = el("span")
            .class("row-title")
            .text(member.display_name.clone());
        if member.is_me {
            title = title.text(" ").child(pill("You", "pill-wait"));
        }
        list = list.child(
            el("li").child(
                el("div").class("row-main").child(title).child(
                    el("span")
                        .class("row-meta")
                        .text(copy::role_name(member.role)),
                ),
            ),
        );
    }
    panel("Members").child(list)
}

fn invitation_name(kind: InvitationKind) -> &'static str {
    match kind {
        InvitationKind::Player => "Player invitation",
        InvitationKind::Contributor => "Contributor invitation",
    }
}

fn invitations_panel(
    screen: &CampaignScreen,
    overview: &CampaignOverview,
    offset_east_minutes: i32,
    is_busy: bool,
) -> Element {
    let mut panel = panel("Invite people")
        .child(el("p").text(
            "A player takes a seat. A contributor joins without a seat, sees the pot totals and \
             can add credit.",
        ))
        .child(actions(vec![
            button(
                "Create player invitation",
                Some(Intent::CreateInvitation(InvitationKind::Player)),
                ButtonKind::Primary,
                is_busy,
            ),
            button(
                "Create contributor invitation",
                Some(Intent::CreateInvitation(InvitationKind::Contributor)),
                ButtonKind::Secondary,
                is_busy,
            ),
        ]));
    if let Some(created) = &screen.created_link {
        panel = panel
            .child(status_line(
                "Invitation link created. This link is shown once; copy it now.",
            ))
            .child(
                el("div")
                    .class("copy-field")
                    .child(el("code").text(created.link.clone()))
                    .child(button(
                        "Copy link",
                        Some(Intent::CopyText(created.link.clone())),
                        ButtonKind::Secondary,
                        false,
                    )),
            );
    }
    panel = panel.child(el("h3").text("Invitations you have sent"));
    if overview.invitations.is_empty() {
        return panel.child(el("p").class("hint").text("No invitations yet."));
    }
    let mut list = el("ul").class("rows");
    for invitation in &overview.invitations {
        list = list.child(invitation_row(invitation, offset_east_minutes, is_busy));
    }
    panel.child(list)
}

fn invitation_row(invitation: &InvitationRow, offset_east_minutes: i32, is_busy: bool) -> Node {
    let name = invitation_name(invitation.kind);
    let expires = money::date_time(invitation.expires_at_ms, offset_east_minutes);
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(el("span").class("row-title").text(name))
                .child(el("span").class("row-meta").text(format!(
                    "Used {} of {} times. Expires {expires}.",
                    invitation.uses, invitation.maximum_uses
                ))),
        )
        .child(row_button(
            "Revoke",
            format!("Revoke {} that expires {expires}", name.to_lowercase()),
            Intent::RevokeInvitation {
                invitation_id: invitation.invitation_id.clone(),
            },
            ButtonKind::Destructive,
            is_busy,
        ))
        .into()
}

fn room_code_panel(room_code: &RoomCodeState, is_busy: bool) -> Element {
    let panel = panel("Room code");
    match room_code {
        RoomCodeState::Off => panel.child(el("p").text(
            "Room codes are off for this campaign. People can only join with an invitation link.",
        )),
        RoomCodeState::Active { code } => panel
            .child(el("p").child(el("span").class("code-box").text(code.clone())))
            .child(el("p").text("People who enter this code ask to join; you approve each one."))
            .child(actions(vec![button(
                "Rotate code",
                Some(Intent::RotateCode),
                ButtonKind::Secondary,
                is_busy,
            )]))
            .child(
                el("p")
                    .class("hint")
                    .text("Rotating makes a new code and stops the old one working."),
            ),
    }
}

fn requests_panel(requests: &[JoinRequestRow], offset_east_minutes: i32, is_busy: bool) -> Element {
    let panel = panel("People asking to join");
    if requests.is_empty() {
        return panel.child(el("p").class("hint").text("Nobody is waiting to join."));
    }
    let mut list = el("ul").class("rows");
    for request in requests {
        list = list.child(request_row(request, offset_east_minutes, is_busy));
    }
    panel.child(list)
}

fn request_row(request: &JoinRequestRow, offset_east_minutes: i32, is_busy: bool) -> Node {
    let who = &request.display_name;
    let decide = |decision| Intent::DecideRequest {
        request_id: request.request_id.clone(),
        decision,
    };
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(el("span").class("row-title").text(who.clone()))
                .child(el("span").class("row-meta").text(format!(
                    "Asked {}",
                    money::date_time(request.requested_at_ms, offset_east_minutes)
                ))),
        )
        .child(
            el("div")
                .class("row-actions")
                .child(row_button(
                    "Approve as player",
                    format!("Approve as player: {who}"),
                    decide(RequestDecision::Approve(InvitationKind::Player)),
                    ButtonKind::Primary,
                    is_busy,
                ))
                .child(row_button(
                    "Approve as contributor",
                    format!("Approve as contributor: {who}"),
                    decide(RequestDecision::Approve(InvitationKind::Contributor)),
                    ButtonKind::Secondary,
                    is_busy,
                ))
                .child(row_button(
                    "Deny",
                    format!("Deny: {who}"),
                    decide(RequestDecision::Deny),
                    ButtonKind::Destructive,
                    is_busy,
                )),
        )
        .into()
}

fn leave_panel(screen: &CampaignScreen, campaign_name: &str, is_busy: bool) -> Element {
    let panel = panel("Leave this campaign");
    if !screen.is_confirming_leave {
        return panel.child(actions(vec![button(
            "Leave campaign",
            Some(Intent::AskLeave),
            ButtonKind::Quiet,
            false,
        )]));
    }
    panel
        .child(
            el("p")
                .attr("role", "alert")
                .text(format!("Leave {campaign_name}? You can be invited again.")),
        )
        .child(actions(vec![
            button(
                "Yes, leave",
                Some(Intent::LeaveCampaign),
                ButtonKind::Destructive,
                is_busy,
            ),
            button(
                "Stay",
                Some(Intent::CancelEdit),
                ButtonKind::Secondary,
                false,
            ),
        ]))
}
