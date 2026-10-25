//! The public pages. Round 1 ships short, honest stubs; Round 4 replaces the home page with the
//! committed `docs/index.html` design and writes the rest (plan 11).

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{
    html::{markup, text},
    layout::{Navigation, page},
    respond::{html, ok_page},
};

const STYLESHEET: &str = include_str!("../assets/portal.css");

pub async fn home() -> Response {
    ok_page(
        "Your table, with a game master",
        Navigation::Visitor,
        markup!(
            "<section class=\"hero\"><h1>DungeonFlux</h1>",
            "<p class=\"lead\">An AI game master for a real table. The TV shows the scene; ",
            "every player plays from their phone.</p>",
            "<p><a class=\"button primary\" href=\"/signup\">Create an account</a> ",
            "<a class=\"button secondary\" href=\"/login\">Log in</a></p>",
            "<p class=\"hint\">Invited to a table? Open the link the host sent you.</p>",
            "</section>"
        ),
    )
}

pub async fn how_it_plays() -> Response {
    ok_page(
        "How a table plays",
        Navigation::Visitor,
        markup!(
            "<h1>How a table plays</h1>",
            "<p>The host starts a table and puts it on a TV. Players scan the code on the TV ",
            "with their phones, make their heroes, and speak or type what they do. The game ",
            "master narrates, voices the people you meet and shows each scene on the TV.</p>",
            "<p>A fuller guide arrives with the business site.</p>"
        ),
    )
}

pub async fn pricing() -> Response {
    ok_page(
        "Pricing",
        Navigation::Visitor,
        markup!(
            "<h1>Pricing</h1>",
            "<p>Play is paid with prepaid credit the host adds to their account. Prices are not ",
            "set yet; this page will explain them from the price table before any payment is ",
            "possible.</p>"
        ),
    )
}

pub async fn faq() -> Response {
    ok_page(
        "Questions",
        Navigation::Visitor,
        markup!(
            "<h1>Questions</h1>",
            "<h2>Does every player pay?</h2>",
            "<p>No. Every player signs in, but only the people who choose to fund a table pay.</p>",
            "<h2>I forgot my password.</h2>",
            "<p>Use <a href=\"/password-reset\">Reset your password</a>; we email you a link that ",
            "works for 15 minutes.</p>"
        ),
    )
}

fn draft(title: &str, heading: &str) -> Response {
    ok_page(
        title,
        Navigation::Visitor,
        markup!(
            "<div class=\"notice warning\" role=\"note\">DRAFT: this page is a placeholder ",
            "and is not yet a legal document.</div><h1>",
            text(heading),
            "</h1><p>The final text will be published here before DungeonFlux takes any payment.</p>"
        ),
    )
}

pub async fn terms() -> Response {
    draft("Terms (draft)", "Terms of service")
}

pub async fn privacy() -> Response {
    draft("Privacy (draft)", "Privacy policy")
}

pub async fn asset(Path(name): Path<String>) -> Response {
    match name.as_str() {
        "portal.css" => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                (header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            STYLESHEET,
        )
            .into_response(),
        _ => html(
            StatusCode::NOT_FOUND,
            page(
                "Not found",
                Navigation::Visitor,
                markup!("<h1>Not found</h1>"),
            ),
        ),
    }
}
