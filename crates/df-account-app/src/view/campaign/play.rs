//! The account's hand-off at `/app/sessions/:id/play`: once the started session's game is reachable,
//! it opens the game launcher (the desktop launcher's screen), which offers the table screen, the
//! players' join and the room code. The account never offers those itself
//! (planning/client-architecture.md "Game start flow", owner decision 2026-10-26).

use super::{actions, panel};
use crate::{
    node::{Element, Node, el},
    route::Route,
    state::{PlayScreen, Remote, SLOW_START_POLLS},
    view::{ButtonKind, button, failed, link, loading, page_wide},
    wire::ErrorCode,
};

pub(in crate::view) fn play(screen: &PlayScreen) -> Node {
    let launch = match &screen.launch {
        Remote::Loading => {
            return page_wide("Starting your game", None)
                .child(loading("Starting your game"))
                .into();
        }
        Remote::Failed(error) => {
            if error.code == ErrorCode::NotFound {
                return page_wide("This game is no longer open", None)
                    .child(
                        el("p").text("This session may have ended or the link may be out of date."),
                    )
                    .child(actions(vec![link(
                        "Back to your games",
                        Route::Dashboard,
                        "btn btn-gilt",
                    )]))
                    .into();
            }
            return page_wide("Your game", None).child(failed(*error)).into();
        }
        Remote::Ready(launch) => launch,
    };
    let body = match (&launch.launcher_url, launch.has_started) {
        (Some(url), _) => ready_panel(url),
        (None, true) if screen.waiting_polls >= SLOW_START_POLLS => {
            panel("Your game is taking longer than usual").child(el("p").text(
                "It opens here as soon as it is ready. If it has not opened in a few minutes, come back and try again.",
            ))
        }
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
    let body = if launch.has_started && !launch.can_manage {
        panel("The host runs the table").child(
            el("p").text("Join from the table screen with the room code the host shares with you."),
        )
    } else if screen.polling_timed_out {
        panel("We couldn't start your game")
            .child(el("p").text("Try again to check whether your game is ready."))
            .child(actions(vec![button(
                "Try again",
                Some(crate::state::Intent::Retry),
                ButtonKind::Primary,
                false,
            )]))
    } else {
        body
    };
    page_wide(&launch.campaign_name, None).child(body).into()
}

fn ready_panel(launcher_url: &str) -> Element {
    panel("Your game is ready")
        .child(el("p").text(
            "The game launcher shows your game, its room code and the heroes as they gather. Open the table screen and let players join from there.",
        ))
        .child(actions(vec![
            el("a")
                .class("btn btn-gilt")
                .attr("href", launcher_url)
                .attr("target", "_blank")
                .attr("rel", "noopener noreferrer")
                .text("Open the game launcher")
                .into(),
        ]))
}
