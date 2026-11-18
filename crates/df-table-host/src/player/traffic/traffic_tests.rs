//! The player listener's tagged answers, its 304s and its traffic counts.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use super::{PlayerTraffic, TrafficCounts};
use crate::guard::Audience;
use crate::test_support::{STARTER_FILES, raw_exchange, start};

/// Serves the player routes over `state` on an ephemeral loopback port, counting in `traffic`.
async fn serve(
    state: Arc<crate::state::AppState>,
    traffic: Arc<PlayerTraffic>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    let audience = Audience::Players {
        addresses: vec![IpAddr::from([192, 168, 1, 30])],
        port,
    };
    let routes = crate::player::routes(state, audience, traffic)
        .into_make_service_with_connect_info::<SocketAddr>();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, routes).await;
    });
    (port, server)
}

fn get(path: &str, port: u16, held: Option<&str>) -> String {
    let condition = held.map_or_else(String::new, |tag| format!("If-None-Match: {tag}\r\n"));
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{condition}Connection: close\r\n\r\n")
}

fn header<'answer>(answer: &'answer str, name: &str) -> Option<&'answer str> {
    let (head, _) = answer.split_once("\r\n\r\n")?;
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn body(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

#[tokio::test]
async fn an_unchanged_json_answer_is_a_bodiless_304_and_a_changed_one_a_new_tag() {
    let running = start(&STARTER_FILES).await;
    let traffic = Arc::new(PlayerTraffic::default());
    let (port, server) = serve(Arc::clone(&running.state), Arc::clone(&traffic)).await;

    let first = raw_exchange(port, &get("/play/room", port, None)).await;
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    let tag = header(&first, "etag").expect("tagged").to_owned();
    assert_eq!(header(&first, "cache-control"), Some("private, no-cache"));
    assert_eq!(header(&first, "vary"), Some("x-df-seat-key"));
    let sent = body(&first).len();

    let again = raw_exchange(port, &get("/play/room", port, Some(&tag))).await;
    assert!(again.starts_with("HTTP/1.1 304"), "{again}");
    assert_eq!(header(&again, "etag"), Some(tag.as_str()));
    assert_eq!(body(&again), "");

    let stale = raw_exchange(port, &get("/play/room", port, Some("\"0-0\""))).await;
    assert!(stale.starts_with("HTTP/1.1 200"), "{stale}");
    assert_eq!(header(&stale, "etag"), Some(tag.as_str()));

    // A page that is not JSON is passed as it came, untagged.
    running.write_asset("index.html", "<!doctype html><p>phone</p>");
    let page = raw_exchange(port, &get("/play", port, None)).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert_eq!(header(&page, "etag"), None);

    let counts = traffic.take();
    assert_eq!(counts.requests, 4);
    assert_eq!(counts.not_modified, 1);
    let page_bytes = body(&page).len();
    assert_eq!(
        counts.bytes_out,
        u64::try_from(sent * 2 + page_bytes).expect("small")
    );
    assert_eq!(traffic.take().requests, 0, "taking starts a new window");
    server.abort();
}

#[test]
fn a_window_with_answers_writes_its_line_and_an_empty_one_writes_none() {
    let counts = TrafficCounts {
        requests: 30,
        not_modified: 25,
        bytes_out: 4_096,
        busy: 2,
        timed_out: 1,
    };
    assert_eq!(
        counts.line(60).as_deref(),
        Some(
            "PLAYER traffic window_s=60 requests=30 not_modified=25 bytes_out=4096 busy=2 \
             timed_out=1"
        )
    );
    let empty = TrafficCounts {
        requests: 0,
        not_modified: 0,
        bytes_out: 0,
        busy: 0,
        timed_out: 0,
    };
    assert_eq!(empty.line(60), None);
}

#[tokio::test]
async fn a_request_past_the_listeners_bound_is_turned_away_at_once() {
    let running = start(&STARTER_FILES).await;
    let traffic = Arc::new(PlayerTraffic::default());
    // Every slot is taken, as by a flood of requests still being answered.
    let held = Arc::clone(&traffic.slots)
        .acquire_many_owned(u32::try_from(crate::limits::MAXIMUM_PLAYER_REQUESTS).expect("small"))
        .await
        .expect("takes every slot");
    let (port, server) = serve(Arc::clone(&running.state), Arc::clone(&traffic)).await;
    let busy = raw_exchange(port, &get("/play/room", port, None)).await;
    assert!(busy.starts_with("HTTP/1.1 503"), "{busy}");
    drop(held);
    let answered = raw_exchange(port, &get("/play/room", port, None)).await;
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    assert_eq!(traffic.take().busy, 1);
    server.abort();
}

#[test]
fn a_request_that_waits_on_work_is_given_the_phones_long_bound_and_any_other_the_short_one() {
    use axum::http::Method;
    let work = std::time::Duration::from_millis(df_playground::limits::WORK_REQUEST_TIMEOUT_MS);
    let short = std::time::Duration::from_millis(df_playground::limits::FRAME_REQUEST_TIMEOUT_MS);
    assert_eq!(super::deadline_of(&Method::POST, "/play/look/refine"), work);
    assert_eq!(
        super::deadline_of(&Method::POST, "/play/seat/transcribe"),
        work
    );
    assert_eq!(super::deadline_of(&Method::POST, "/play/seat/action"), work);
    assert_eq!(super::deadline_of(&Method::GET, "/play/look"), short);
    assert_eq!(super::deadline_of(&Method::GET, "/playground/frame"), short);
    assert_eq!(super::deadline_of(&Method::POST, "/play/seat/say"), short);
}
