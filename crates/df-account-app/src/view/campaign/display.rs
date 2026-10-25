//! The TV's pairing screen at `/app/display` (S1 8.5): large type, readable across a room. It
//! holds no secret: the device secret and the ticket are cookies the browser keeps to itself.

use crate::{
    copy,
    node::{Node, el},
    state::{DisplayEnd, DisplayPhase, DisplayScreen, Intent},
    view::{ButtonKind, button, loading, page_wide},
};

pub(in crate::view) fn display(screen: &DisplayScreen, now_ms: u64) -> Node {
    let shell = page_wide("Show this screen on your TV", None);
    let body = match &screen.phase {
        DisplayPhase::Idle => idle(),
        DisplayPhase::Requesting => loading("Getting a code"),
        DisplayPhase::Showing {
            user_code,
            expires_at_ms,
        } => showing(user_code, expires_at_ms.saturating_sub(now_ms)),
        DisplayPhase::Redeeming { .. } | DisplayPhase::Opening => connected(),
        DisplayPhase::Ended(end) => ended(end),
    };
    el("div")
        .class("display-screen")
        .child(shell.child(body))
        .into()
}

fn show_code_button() -> Node {
    el("p")
        .class("actions")
        .child(button(
            "Show pairing code",
            Some(Intent::ShowPairingCode),
            ButtonKind::Primary,
            false,
        ))
        .into()
}

fn idle() -> Node {
    el("div")
        .child(el("p").class("lede").text(
            "Open this page on the TV or computer that will show the game. It shows a short \
             code. You type that code on your phone, in the lobby, to connect this screen to \
             your table.",
        ))
        .child(show_code_button())
        .into()
}

fn showing(user_code: &str, remaining_ms: u64) -> Node {
    el("div")
        .child(
            el("p")
                .class("display-code-line")
                .child(el("span").class("code-box display-code").text(user_code)),
        )
        .child(
            el("p")
                .class("lede")
                .text("On your phone, open the lobby, choose Pair a display and type this code"),
        )
        .child(el("p").text(expiry_text(remaining_ms)))
        .child(
            el("p")
                .class("display-waiting")
                .attr("role", "status")
                .attr("aria-live", "polite")
                .text("Waiting for the host to confirm"),
        )
        .into()
}

/// Relative, so a TV with the wrong time zone still reads right.
fn expiry_text(remaining_ms: u64) -> String {
    match remaining_ms.div_ceil(60_000) {
        0 => "This code has expired.".to_owned(),
        1 => "This code stops working in about 1 minute.".to_owned(),
        minutes => format!("This code stops working in about {minutes} minutes."),
    }
}

fn connected() -> Node {
    el("p")
        .class("lede")
        .attr("role", "status")
        .attr("aria-live", "polite")
        .text("Connected. Opening the game display...")
        .into()
}

fn ended(end: &DisplayEnd) -> Node {
    let (what, fix) = match end {
        DisplayEnd::Expired => (
            "This code expired.",
            "Show a new code, then type it in the lobby.",
        ),
        DisplayEnd::Cancelled => (
            "The pairing was cancelled.",
            "Show a new code if you still want to use this screen.",
        ),
        // The screen limit is a reason of its own, and `problem` words it.
        DisplayEnd::Refused(error) => {
            let problem = copy::problem(*error);
            (problem.what, problem.fix)
        }
    };
    el("div")
        .child(
            el("div")
                .class("problem")
                .attr("role", "alert")
                .child(el("p").class("problem-what").text(what))
                .child(el("p").text(fix)),
        )
        .child(el("p").class("actions").child(button(
            "Show a new code",
            Some(Intent::ShowPairingCode),
            ButtonKind::Primary,
            false,
        )))
        .into()
}
