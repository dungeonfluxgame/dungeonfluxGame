//! The player listener: its allow-list, its guard, a phone's report and the phone link page.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::command::split_words;
use crate::dev::run_dev;
use crate::guard::Audience;
use crate::player::{self, phone_page_link};
use crate::state::AppState;
use crate::test_support::{Running, STARTER_FILES, raw_exchange, start};
use df_playground::frame::Frame;

/// The network address the tests' player listener answers to.
const LAN: [u8; 4] = [192, 168, 1, 30];

/// Serves the player router over `state` on an ephemeral loopback port, answering `LAN` on that
/// port, with each connection's address as the real listener gives it.
async fn serve_players(state: Arc<AppState>) -> (u16, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    let audience = Audience::Players {
        addresses: vec![IpAddr::from(LAN)],
        port,
    };
    let routes =
        player::router(state, audience).into_make_service_with_connect_info::<SocketAddr>();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, routes).await;
    });
    (port, server)
}

fn lan_host(port: u16) -> String {
    format!("{}:{port}", IpAddr::from(LAN))
}

fn get_request(path: &str, host: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

fn post_request(path: &str, host: &str, origin: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn status(answer: &str) -> &str {
    answer.split(' ').nth(1).unwrap_or("")
}

fn log_text(running: &Running) -> String {
    running
        .state
        .log()
        .newest(1_000)
        .iter()
        .map(|event| event.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn the_player_listener_serves_its_allow_list_and_nothing_else() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("index.html", "<!doctype html><title>page</title>");
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    for path in ["/play", "/playground/frame"] {
        let answer = raw_exchange(port, &get_request(path, &host)).await;
        assert_eq!(status(&answer), "200", "{path}: {answer}");
    }
    for path in [
        "/playground/controls.json",
        "/playground/revision",
        "/canvas.json",
        "/events.json",
        "/",
        "/playground",
        "/playground/clients.json",
        "/dev/host/status",
        "/dev/persona",
        "/dev/speech/status",
        "/playground/phone-link",
    ] {
        let answer = raw_exchange(port, &get_request(path, &host)).await;
        assert_eq!(status(&answer), "404", "{path}: {answer}");
    }
    let origin = format!("http://{host}");
    for path in [
        "/dev/effect",
        "/dev/reload",
        "/dev/playground/controls",
        "/dev/playground/scene",
        "/dev/persona/ask",
    ] {
        let answer = raw_exchange(port, &post_request(path, &host, &origin, "{}")).await;
        assert_eq!(status(&answer), "404", "{path}: {answer}");
    }
    server.abort();
}

#[tokio::test]
async fn a_phones_report_is_recorded_with_its_role_and_address() {
    let running = start(&STARTER_FILES).await;
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    let body = serde_json::json!({
        "client": "phone-0000beef",
        "report": df_playground::media::ClientReport {
            role: df_playground::role::PageRole::Phone,
            ..df_playground::media::ClientReport::default()
        },
    })
    .to_string();
    let answer = raw_exchange(
        port,
        &post_request(
            df_playground::path::CLIENT_REPORT,
            &host,
            &format!("http://{host}"),
            &body,
        ),
    )
    .await;
    assert_eq!(status(&answer), "200", "{answer}");
    let log = log_text(&running);
    assert!(
        log.contains(
            "CLIENT seen client=phone-0000beef device=both viewport=- role=phone remote=127.0.0.1"
        ),
        "{log}"
    );
    server.abort();
}

#[tokio::test]
async fn a_foreign_host_and_a_cross_site_report_are_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    for foreign in [
        format!("evil.example:{port}"),
        format!("192.168.1.31:{port}"),
    ] {
        let answer = raw_exchange(port, &get_request("/playground/controls.json", &foreign)).await;
        assert_eq!(status(&answer), "403", "{foreign}: {answer}");
    }
    let answer = raw_exchange(
        port,
        &post_request(
            df_playground::path::CLIENT_REPORT,
            &host,
            "http://evil.example",
            "{}",
        ),
    )
    .await;
    assert_eq!(status(&answer), "403", "{answer}");
    let log = log_text(&running);
    assert_eq!(
        log.matches("HTTP refused method=get route=unmatched cause=forbidden_request")
            .count(),
        2,
        "{log}"
    );
    assert!(
        log.contains("HTTP refused method=post route=unmatched cause=forbidden_request"),
        "{log}"
    );
    server.abort();
}

#[tokio::test]
async fn the_phone_link_page_says_how_to_start_players_then_shows_the_link_and_its_code() {
    let running = start(&STARTER_FILES).await;
    let page = player::path::PHONE_LINK;
    let off = raw_exchange(running.port, &get_request(page, "127.0.0.1")).await;
    assert_eq!(status(&off), "200", "{off}");
    assert!(off.contains("--players"), "{off}");
    assert!(!off.contains("<svg"), "{off}");
    let link = phone_page_link(IpAddr::from(LAN), 7880);
    assert_eq!(link, "http://192.168.1.30:7880/play");
    running.state.set_player_links(vec![link.clone()]);
    let on = raw_exchange(running.port, &get_request(page, "127.0.0.1")).await;
    assert!(on.contains(&link), "{on}");
    assert!(on.contains("<svg"), "{on}");
    let status_text =
        raw_exchange(running.port, &get_request("/dev/host/status", "127.0.0.1")).await;
    assert!(status_text.contains(&link), "{status_text}");
}

#[tokio::test]
async fn a_phone_is_served_a_media_file_only_with_a_link_signed_for_it() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "png bytes");
    running.write_asset("media/other.png", "png bytes");
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs();
    let link = running
        .state
        .media_links()
        .link("plate.png", now)
        .expect("the host has a secret");
    let fetch = |path: String| {
        let request = get_request(&path, &host);
        async move { raw_exchange(port, &request).await }
    };
    let unsigned = fetch("/playground/media/plate.png".to_owned()).await;
    assert_eq!(status(&unsigned), "404", "{unsigned}");
    let signed = fetch(format!("/playground/media/plate.png?ml={link}")).await;
    assert_eq!(status(&signed), "200", "{signed}");
    let borrowed = fetch(format!("/playground/media/other.png?ml={link}")).await;
    assert_eq!(status(&borrowed), "404", "a link is for its own file only");
    let expired = running
        .state
        .media_links()
        .link("plate.png", 0)
        .expect("signed");
    let late = fetch(format!("/playground/media/plate.png?ml={expired}")).await;
    assert_eq!(status(&late), "404", "{late}");
    let log = log_text(&running);
    assert!(log.contains("HOST media_refused reason=unsigned"), "{log}");
    assert!(
        log.contains("HOST media_refused reason=bad_signature"),
        "{log}"
    );
    assert!(log.contains("HOST media_refused reason=expired"), "{log}");
    // TODO(DEV-12): a seat answer naming a library file carries its signed link in the
    // `x-df-media-links` header, and the phone's media addresses use it.
    server.abort();
}

#[tokio::test]
async fn a_phones_change_wakes_the_frame_polls_and_its_report_does_not() {
    let running = start(&STARTER_FILES).await;
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    let origin = format!("http://{host}");
    let before = running.state.change_count();
    let report = serde_json::json!({
        "client": "phone-0000beef",
        "report": df_playground::media::ClientReport::default(),
    })
    .to_string();
    let reported = raw_exchange(
        port,
        &post_request(df_playground::path::CLIENT_REPORT, &host, &origin, &report),
    )
    .await;
    assert_eq!(status(&reported), "200", "{reported}");
    assert_eq!(
        running.state.change_count(),
        before,
        "a report is a reading"
    );
    // A change attempt wakes the pages whether the route takes it or not, as on the dev API.
    let said = raw_exchange(
        port,
        &post_request(df_playground::path::PLAY_HELLO, &host, &origin, "{}"),
    )
    .await;
    assert!(!status(&said).is_empty(), "{said}");
    assert!(
        running.state.change_count() > before,
        "a phone's change is announced"
    );
    server.abort();
}

#[tokio::test]
async fn a_phone_is_sent_the_frame_as_a_phone_draws_it_and_the_shared_screen_all_of_it() {
    let running = start(&STARTER_FILES).await;
    run_dev(
        running.port,
        &split_words("spawn marker m1").expect("the line splits"),
    )
    .await
    .expect("spawns");
    let (port, server) = serve_players(Arc::clone(&running.state)).await;
    let host = lan_host(port);
    let answer = raw_exchange(port, &get_request("/playground/frame", &host)).await;
    let (_, body) = answer.split_once("\r\n\r\n").expect("an answer");
    let phone: Frame = serde_json::from_str(body).expect("a frame");
    let shared: Frame = serde_json::from_str(
        &crate::client::get(running.port, "/playground/frame")
            .await
            .expect("answers")
            .body,
    )
    .expect("a frame");
    assert_eq!(shared.canvas.elements.len(), 1);
    assert!(phone.canvas.elements.is_empty(), "a phone draws no stage");
    assert_eq!(phone.canvas.revision, shared.canvas.revision);
    assert_eq!(phone.frame, shared.frame);
    assert_eq!(phone.asset_revision, shared.asset_revision);
    server.abort();
}
