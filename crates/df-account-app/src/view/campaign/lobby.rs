//! Owner decision 2026-10-26: the website owns membership and funding; the game owns its room.

use super::{actions, is_busy, panel, pill};
#[cfg(test)]
#[path = "funding_tests.rs"]
mod funding_tests;
use crate::{
    copy, money,
    node::{Element, Node, el},
    route::Route,
    state::{Field, Form, Intent, LobbyScreen, Remote},
    view::{ButtonKind, button, checkbox, error_summary, failed, link, loading, page_wide},
    wire_campaign::{FundingNote, LobbyMember, LobbyView, StartBlocker},
};

pub(in crate::view) fn lobby(screen: &LobbyScreen, _offset_east_minutes: i32) -> Node {
    let lobby = match &screen.lobby {
        Remote::Loading => {
            return page_wide("Session", None)
                .child(loading("Opening the session"))
                .into();
        }
        Remote::Failed(error) => return page_wide("Session", None).child(failed(*error)).into(),
        Remote::Ready(lobby) => lobby,
    };
    let busy = is_busy(screen.action, &screen.operation);
    let mut sections = el("div").class("stack");
    if lobby.has_started {
        sections = sections.child(started_panel(&screen.session_id));
    }
    sections = sections.child(members_panel(lobby));
    // The pre-start cost wording ("No charge until ...") is wrong once the session runs.
    if !lobby.has_started {
        sections = sections.child(funding_panel(lobby.funding_note, screen, busy));
        if lobby.can_manage {
            let mut start = panel("Start the session");
            if !lobby.start_blockers.is_empty() {
                start = start.child(blockers_list(&lobby.start_blockers));
            }
            start = start.child(actions(vec![
                button(
                    "Start the session",
                    Some(Intent::StartSession),
                    ButtonKind::Primary,
                    busy || !lobby.start_blockers.is_empty(),
                ),
                button(
                    "Cancel",
                    Some(Intent::CancelLobby),
                    ButtonKind::Secondary,
                    busy,
                ),
            ]));
            sections = sections.child(start);
        } else {
            sections = sections.child(
                panel("Join the game").child(el("p").text("Waiting for the host to start.")),
            );
        }
    }
    page_wide(&lobby.campaign_name, None)
        .child(error_summary(screen.action.error()))
        .child(el("p").class("hint").text("This page updates by itself."))
        .child(sections)
        .into()
}

fn started_panel(session_id: &str) -> Element {
    panel("The session has started").child(actions(vec![link(
        "Open the game",
        Route::Play {
            session_id: session_id.to_owned(),
        },
        "btn btn-gilt",
    )]))
}

fn members_panel(lobby: &LobbyView) -> Element {
    panel("Who has joined").child(
        el("ul")
            .class("rows")
            .children(lobby.members.iter().map(member_row)),
    )
}

fn member_row(member: &LobbyMember) -> Node {
    let mut title = el("span")
        .class("row-title")
        .text(member.display_name.clone());
    if member.is_me {
        title = title.text(" ").child(pill("You", "pill-wait"));
    }
    el("li").child(title).into()
}

fn funding_panel(note: FundingNote, screen: &LobbyScreen, is_busy: bool) -> Element {
    let sentence = match note {
        FundingNote::NoChargeUntilHostStarts => {
            "No charge until the host starts, except setup jobs you approved."
        }
        FundingNote::WaitingForSponsor => "Waiting for the sponsor's funding approval.",
        FundingNote::HostCredit(quote) => {
            let mut cost = panel("Use your credit")
                .child(el("p").text(format!(
                    "Table price: {} per table-hour, at most. Available credit: {}.",
                    money::charge(quote.hourly_micro_dollars),
                    money::balance(quote.available_micro_dollars)
                )))
                .child(el("p").text("No charge until you start the session."));
            if quote.consented_at_ms.is_some() {
                cost = cost.child(el("p").text(format!(
                    "Credit use approved, up to {} for this session.",
                    money::charge(quote.cap_micro_dollars)
                )));
            } else {
                cost = cost.child(
                    el("div")
                        .class("stack")
                        .child(checkbox(
                            Field::FundingChoice,
                            &format!(
                                "Use my credit for this session, up to {}",
                                money::charge(quote.cap_micro_dollars)
                            ),
                            screen.consent_checked,
                            screen.action.error(),
                        ))
                        .child(button(
                            "Confirm credit use",
                            Some(Intent::Submit(Form::Action)),
                            ButtonKind::Primary,
                            is_busy || !screen.consent_checked,
                        )),
                );
            }
            if !quote.is_fundable && quote.available_micro_dollars < quote.cap_micro_dollars {
                cost =
                    cost.child(el("p").text("Add credit to cover the first hour before starting."));
            } else if quote.consented_at_ms.is_none() {
                cost = cost.child(el("p").text("Approve use of your credit before starting."));
            }
            return cost;
        }
    };
    panel("Cost").child(el("p").text(sentence))
}

fn blockers_list(blockers: &[StartBlocker]) -> Element {
    el("ul").attr("id", "start-blockers").children(
        blockers
            .iter()
            .map(|blocker| el("li").text(copy::start_blocker(*blocker)).into()),
    )
}
