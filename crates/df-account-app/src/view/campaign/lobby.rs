//! The lobby at `/app/sessions/:id/lobby`: who is here, what blocks the start, each person's own
//! readiness and microphone, and the host's start and display pairing (plan 9.6; S1 8.4, 8.5).

use super::{actions, is_busy, panel, pill, row_button};
use crate::{
    copy, money,
    node::{Element, Node, el},
    state::{Field, Form, Intent, LobbyScreen, Remote},
    view::{
        ButtonKind, InputKind, TextField, button, error_summary, fact, facts, failed, form,
        loading, page_wide, submit_button, text_field,
    },
    wire_campaign::{
        DisplayRow, FundingNote, LobbyMember, LobbyView, MicrophoneState, PairingLookup,
        StartBlocker,
    },
};

pub(in crate::view) fn lobby(screen: &LobbyScreen, offset_east_minutes: i32) -> Node {
    let lobby = match &screen.lobby {
        Remote::Loading => {
            return page_wide("Lobby", None)
                .child(loading("Opening the lobby"))
                .into();
        }
        Remote::Failed(error) => return page_wide("Lobby", None).child(failed(*error)).into(),
        Remote::Ready(lobby) => lobby,
    };
    let is_busy = is_busy(screen.action, &screen.operation);
    let mut sections = el("div").class("stack");
    if lobby.has_started {
        sections = sections.child(started_panel(&screen.session_id));
    }
    sections = sections
        .child(members_panel(lobby))
        .child(funding_panel(lobby.funding_note));
    if let Some(me) = lobby.members.iter().find(|member| member.is_me) {
        sections = sections.child(own_controls_panel(me, is_busy));
    }
    if !lobby.has_started {
        sections = sections.child(if lobby.can_manage {
            start_panel(&lobby.start_blockers, is_busy)
        } else {
            waiting_panel(&lobby.start_blockers)
        });
    }
    if lobby.can_manage {
        sections = sections.child(pairing_panel(screen, lobby, offset_east_minutes, is_busy));
    }
    let mut shell = page_wide(&format!("Lobby: {}", lobby.campaign_name), None)
        .child(error_summary(screen.action.error()));
    if !lobby.has_started {
        shell = shell.child(el("p").class("hint").text("This page updates by itself."));
    }
    shell.child(sections).into()
}

fn started_panel(session_id: &str) -> Element {
    panel("The session has started").child(
        el("p").class("actions").child(
            // A full page load: the game runs outside this app.
            el("a")
                .class("btn btn-gilt")
                .attr("href", format!("/sessions/{session_id}/play"))
                .text("Open the game"),
        ),
    )
}

fn members_panel(lobby: &LobbyView) -> Element {
    let mut list = el("ul").class("rows");
    for member in &lobby.members {
        list = list.child(member_row(member));
    }
    panel("Who is here").child(list)
}

fn member_row(member: &LobbyMember) -> Node {
    let mut title = el("span")
        .class("row-title")
        .text(member.display_name.clone());
    if member.is_me {
        title = title.text(" ").child(pill("You", "pill-wait"));
    }
    let hero = member
        .character_name
        .as_ref()
        .map_or_else(|| "No hero yet".to_owned(), |name| format!("Hero: {name}"));
    let readiness = if member.is_ready {
        pill("Ready", "pill-ok")
    } else {
        pill("Not ready", "pill-warn")
    };
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(title)
                .child(el("span").class("row-meta member-detail").text(hero))
                .child(
                    el("span")
                        .class("row-meta member-detail")
                        .text(copy::microphone_state(member.microphone)),
                ),
        )
        .child(readiness)
        .into()
}

fn funding_panel(note: FundingNote) -> Element {
    let sentence = match note {
        FundingNote::NoChargeUntilHostStarts => {
            "No charge until the host starts, except setup jobs you approved."
        }
        FundingNote::WaitingForSponsor => "Waiting for the sponsor's funding approval.",
    };
    panel("Cost").child(el("p").text(sentence))
}

fn own_controls_panel(me: &LobbyMember, is_busy: bool) -> Element {
    let (ready_label, ready_intent) = if me.is_ready {
        ("I am not ready", Intent::SetReady(false))
    } else {
        ("I am ready", Intent::SetReady(true))
    };
    let can_turn_on = matches!(
        me.microphone,
        MicrophoneState::NotAsked | MicrophoneState::Declined | MicrophoneState::Typing
    );
    let can_type_instead = matches!(
        me.microphone,
        MicrophoneState::NotAsked | MicrophoneState::Granted
    );
    let mut microphone = Vec::new();
    if can_turn_on {
        microphone.push(button(
            "Turn on my microphone",
            Some(Intent::EnableMicrophone),
            ButtonKind::Secondary,
            is_busy,
        ));
    }
    if can_type_instead {
        microphone.push(button(
            "I will type instead",
            Some(Intent::PreferTyping),
            ButtonKind::Secondary,
            is_busy,
        ));
    }
    let mut panel = panel("Your seat")
        .child(actions(vec![button(
            ready_label,
            Some(ready_intent),
            ButtonKind::Primary,
            is_busy,
        )]))
        .child(el("h3").text("Microphone"))
        .child(el("p").text(copy::microphone_state(me.microphone)))
        .child(el("p").class("hint").text(
            "Your browser will ask for permission to use the microphone. If you say no, or \
             prefer to type, you can still play by typing.",
        ));
    if !microphone.is_empty() {
        panel = panel.child(actions(microphone));
    }
    panel
}

fn blockers_list(blockers: &[StartBlocker]) -> Element {
    let mut list = el("ul").attr("id", "start-blockers");
    for blocker in blockers {
        list = list.child(el("li").text(copy::start_blocker(*blocker)));
    }
    list
}

fn start_panel(blockers: &[StartBlocker], is_busy: bool) -> Element {
    let mut panel = panel("Start the session");
    let can_start = blockers.is_empty();
    if can_start {
        panel = panel.child(el("p").text("Everyone is ready. Start when you are."));
    } else {
        panel = panel
            .child(el("p").text("The session cannot start yet:"))
            .child(blockers_list(blockers));
    }
    let mut start = el("button")
        .class(ButtonKind::Primary.class())
        .attr("type", "button")
        .flag("disabled", is_busy || !can_start)
        .on_click(Intent::StartSession)
        .text("Start the session");
    if is_busy {
        start = start.attr("aria-busy", "true");
    }
    if !can_start {
        start = start.attr("aria-describedby", "start-blockers");
    }
    panel.child(actions(vec![start.into()]))
}

fn waiting_panel(blockers: &[StartBlocker]) -> Element {
    let panel = panel("Before the session can start");
    if blockers.is_empty() {
        return panel.child(el("p").text("Waiting for the host to start the session."));
    }
    panel.child(blockers_list(blockers))
}

fn pairing_panel(
    screen: &LobbyScreen,
    lobby: &LobbyView,
    offset_east_minutes: i32,
    is_busy: bool,
) -> Element {
    let panel = panel("Pair a display").child(el("p").text(
        "On the TV or computer you want to use, open the page at /app/display and choose Show \
         pairing code. Then type the code it shows here.",
    ));
    let panel = match &screen.lookup {
        Some(PairingLookup::Found {
            device,
            created_at_ms,
            waiting_seconds,
            ..
        }) => panel.child(found_screen(
            &screen.pairing_code,
            facts(vec![
                fact("Device", copy::device_name(*device)),
                fact("Waiting for", money::duration(*waiting_seconds)),
                fact(
                    "Asked at",
                    money::date_time(*created_at_ms, offset_east_minutes),
                ),
            ]),
            is_busy,
        )),
        Some(PairingLookup::NotFound) => panel
            .child(el("p").class("problem-what").attr("role", "alert").text(
                "No screen is showing that code. Check the code on the screen and type it \
                     again. A code stops working after a few minutes.",
            ))
            .child(lookup_form(screen)),
        None => panel.child(lookup_form(screen)),
    };
    panel
        .child(el("h3").text("Paired screens"))
        .child(displays_list(&lobby.displays, offset_east_minutes, is_busy))
}

fn lookup_form(screen: &LobbyScreen) -> Node {
    form(Form::PairingLookup, "Pair a display")
        .child(text_field(
            TextField {
                field: Field::PairingCode,
                label: "Pairing code",
                value: &screen.pairing_code,
                kind: InputKind::Text,
                autocomplete: "off",
                hint: Some("The code on the screen, like ABCD-EFGH."),
            },
            screen.action.error(),
        ))
        .child(submit_button(
            "Look up screen",
            "Looking up...",
            screen.action.is_sending(),
        ))
        .into()
}

fn found_screen(typed_code: &str, details: Node, is_busy: bool) -> Node {
    el("div")
        .class("pairing-found")
        .child(
            el("p")
                .class("pairing-warning")
                .child(el("strong").text("Only pair a screen you can see showing this code"))
                .text(": ")
                .child(el("span").class("code-box code-small").text(typed_code)),
        )
        .child(details)
        .child(actions(vec![
            button(
                "Confirm: pair this screen",
                Some(Intent::ApprovePairing),
                ButtonKind::Primary,
                is_busy,
            ),
            button(
                "Cancel",
                Some(Intent::CancelPairing),
                ButtonKind::Secondary,
                false,
            ),
        ]))
        .into()
}

fn displays_list(displays: &[DisplayRow], offset_east_minutes: i32, is_busy: bool) -> Node {
    if displays.is_empty() {
        return el("p")
            .class("hint")
            .text("No screens are paired yet.")
            .into();
    }
    let mut list = el("ul").class("rows");
    for display in displays {
        let device = copy::device_name(display.device);
        let paired = money::date_time(display.paired_at_ms, offset_east_minutes);
        list = list.child(
            el("li")
                .child(
                    el("div")
                        .class("row-main")
                        .child(el("span").class("row-title").text(device))
                        .child(
                            el("span")
                                .class("row-meta")
                                .text(format!("Paired {paired}")),
                        ),
                )
                .child(row_button(
                    "Unpair",
                    format!("Unpair {device} paired {paired}"),
                    Intent::UnpairDisplay {
                        ticket_id: display.ticket_id.clone(),
                    },
                    ButtonKind::Destructive,
                    is_busy,
                )),
        );
    }
    el("div")
        .child(list)
        .child(actions(vec![button(
            "Unpair all",
            Some(Intent::UnpairAllDisplays),
            ButtonKind::Destructive,
            is_busy,
        )]))
        .into()
}
