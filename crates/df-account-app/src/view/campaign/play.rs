//! The pre-game launcher at `/app/sessions/:id/play`: after the website starts a session, it opens
//! the game's table screen and player screen. The game owns its room (owner decision 2026-10-26).

use super::{actions, panel};
use crate::{
    node::{Element, Node, el},
    route::Route,
    state::{PlayScreen, Remote},
    view::{failed, link, loading, page_wide},
    wire_campaign::GameLinks,
};

pub(in crate::view) fn play(screen: &PlayScreen) -> Node {
    let launch = match &screen.launch {
        Remote::Loading => {
            return page_wide("Starting your game", None)
                .child(loading("Starting your game"))
                .into();
        }
        Remote::Failed(error) => {
            return page_wide("Starting your game", None)
                .child(failed(*error))
                .into();
        }
        Remote::Ready(launch) => launch,
    };
    let body = match (&launch.links, launch.has_started) {
        (Some(links), _) => ready_panel(links),
        (None, true) => panel("Your game is starting").child(
            el("p")
                .class("hint")
                .text("This takes a few moments. This page updates by itself."),
        ),
        (None, false) => panel("This session has not started yet").child(actions(vec![link(
            "Go to the session",
            Route::Lobby {
                session_id: screen.session_id.clone(),
            },
            "btn btn-outline",
        )])),
    };
    page_wide(&launch.campaign_name, None).child(body).into()
}

fn ready_panel(links: &GameLinks) -> Element {
    let mut ready = panel("Your game is ready")
        .child(el("p").text(
            "Open the table screen on a TV or any screen everyone can see. Then join as a player here, or on a phone.",
        ))
        .child(actions(vec![
            new_tab("Open the table screen", &links.display_url, "btn btn-gilt"),
            new_tab("Join as a player", &links.player_url, "btn btn-outline"),
        ]));
    if let Some(code) = &links.room_code {
        ready = ready
            .child(el("p").text("On a phone, open the player screen and enter this code:"))
            .child(el("p").child(el("span").class("code-box").text(code.clone())));
    }
    ready
}

fn new_tab(label: &str, url: &str, class: &str) -> Node {
    el("a")
        .class(class)
        .attr("href", url)
        .attr("target", "_blank")
        .attr("rel", "noopener noreferrer")
        .text(label)
        .into()
}
