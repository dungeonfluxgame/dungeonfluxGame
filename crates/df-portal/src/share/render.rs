//! Script-free, escape-by-default public HTML. No media is accepted without a public grant.
use super::PublicSummary;
use crate::{
    html::{Markup, text},
    markup,
};
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
pub(super) const STYLE: &str = include_str!("style.css");
fn headers(response: &mut Response, embeds: bool, indexing: bool) {
    let h = response.headers_mut();
    // No shared caching: immediate withdrawal is enforced without a CDN purge prerequisite.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    h.insert(
        "x-robots-tag",
        HeaderValue::from_static(if indexing {
            "index, follow"
        } else {
            "noindex, nofollow, noarchive"
        }),
    );
    h.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_static(if embeds {"default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors https:"} else {"default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'"}));
}
pub(super) fn ensure_headers(response:&mut Response){
    if !response.headers().contains_key(header::CONTENT_SECURITY_POLICY){headers(response,false,false);}
}
pub(super) fn missing(status: StatusCode) -> Response {
    let mut response=(status,Html("<!doctype html><title>Share unavailable</title><h1>Share unavailable</h1><p>This link is unavailable.</p>")).into_response();
    headers(&mut response, false, false);
    response
}
pub(super) fn page(
    s: &PublicSummary,
    origin: &str,
    token: &str,
    embed: bool,
    theme: &str,
) -> Response {
    let theme = match theme {
        "light" => "light",
        "dark" => "dark",
        _ => "auto",
    };
    let url = format!("{origin}/s/{token}");
    let names = Markup::concat(s.names.iter().map(|n| markup!("<li>", text(n), "</li>")));
    let highlights = Markup::concat(
        s.highlights
            .iter()
            .map(|n| markup!("<li>", text(n), "</li>")),
    );
    let link = if embed {
        markup!(
            "<p><a href=\"",
            text(&url),
            "\" target=\"_blank\" rel=\"noopener noreferrer\">View session summary</a></p>"
        )
    } else {
        Markup::default()
    };
    let html = markup!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>",
        text(&s.title),
        "</title><meta property=\"og:title\" content=\"",
        text(&s.title),
        "\"><meta property=\"og:type\" content=\"article\"><meta property=\"og:url\" content=\"",
        text(&url),
        "\"><meta name=\"twitter:card\" content=\"summary\"><link rel=\"stylesheet\" href=\"/share.css\"></head><body class=\"",
        text(theme),
        "\"><main><h1>",
        text(&s.title),
        "</h1><p>",
        text(&s.date),
        " &middot; about ",
        s.duration_minutes,
        " minutes</p><ul>",
        names,
        "</ul><ul>",
        highlights,
        "</ul>",
        link,
        "</main></body></html>"
    );
    let mut response = Html(html.into_string()).into_response();
    headers(&mut response, s.allow_embeds, s.allow_indexing && !embed);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_ancestors_requires_explicit_embed_opt_in() {
        let mut response = missing(StatusCode::NOT_FOUND);
        headers(&mut response, false, false);
        assert!(
            response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'self'")
        );
        headers(&mut response, true, true);
        assert!(
            response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("frame-ancestors https:")
        );
        assert_eq!(response.headers()["x-robots-tag"], "index, follow");
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    }
}
