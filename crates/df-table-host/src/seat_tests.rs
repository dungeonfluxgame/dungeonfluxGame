//! Seats for phones over the loopback listener and the player listener: the three kinds of
//! player, the seat taken from the key, the lobby joined as that seat, the books kept on disk.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use df_playground::path::{
    PLAY_HELLO, PLAY_HERO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_STATUS, PLAY_ROOM, PLAY_SEAT_INTENT,
    PLAY_SEAT_VIEW, SEAT_KEY_HEADER,
};
use df_playground::seat::wire::{SeatErrorKind, hex_digits};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::engine::request::AudienceSpec;
use crate::guard::Audience;
use crate::seat::SeatDesk;
use crate::seat::book::Holder;
use crate::seat::key::KeyHash;
use crate::seat::routes::path;
use crate::seat::table;
use crate::test_support::{Running, STARTER_FILES, raw_exchange, start};

fn key(byte: u8) -> String {
    hex_digits(&[byte; 32])
}

fn request(method: &str, path: &str, seat_key: Option<&str>, body: Option<&str>) -> String {
    let key_line = seat_key.map_or_else(String::new, |key| format!("{SEAT_KEY_HEADER}: {key}\r\n"));
    let body_lines = body.map_or_else(String::new, |body| {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
    });
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{key_line}{body_lines}\
         Connection: close\r\n\r\n{}",
        body.unwrap_or("")
    )
}

/// The status and the JSON body of one exchange with the loopback listener.
async fn call(
    running: &Running,
    method: &str,
    path: &str,
    seat_key: Option<&str>,
    body: Option<&str>,
) -> (String, Value) {
    let answer = raw_exchange(running.port, &request(method, path, seat_key, body)).await;
    let status = answer.split(' ').nth(1).unwrap_or("").to_owned();
    let json = answer
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    (status, json)
}

fn log_text(running: &Running) -> String {
    running
        .state
        .log()
        .newest(10_000)
        .iter()
        .map(|event| event.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn join_as_guest(running: &Running, seat_key: &str, name: &str) -> (String, Value) {
    let body = serde_json::json!({ "kind": "guest", "name": name }).to_string();
    call(running, "POST", PLAY_JOIN, Some(seat_key), Some(&body)).await
}

async fn is_seated_at_table(running: &Running, seat: u64) -> bool {
    let engine = running.state.engine().expect("the dev engine runs");
    let view = engine
        .intent_view(AudienceSpec::Seat { seat })
        .await
        .expect("a seat view");
    table::is_seated(&view)
}

#[tokio::test]
async fn a_guest_takes_the_first_free_seat_through_the_lobby_and_hello_finds_it_again() {
    let running = start(&STARTER_FILES).await;
    let phone = key(1);

    let (status, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(status, "200", "{hello}");
    assert_eq!(hello["value"]["outcome"], "unknown");

    let (status, joined) = join_as_guest(&running, &phone, "Mara Vell").await;
    assert_eq!(status, "200", "{joined}");
    assert_eq!(joined["value"]["outcome"], "seated");
    assert_eq!(joined["value"]["seat"]["seat"], 1);
    assert_eq!(joined["value"]["seat"]["kind"], "guest");
    assert!(is_seated_at_table(&running, 1).await);

    let (_, again) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(again["value"]["outcome"], "seated");
    assert_eq!(again["value"]["seat"]["name"], "Mara Vell");

    let (_, second) = join_as_guest(&running, &key(2), "Danny").await;
    assert_eq!(second["value"]["seat"]["seat"], 2);
    let (_, room) = call(&running, "GET", PLAY_ROOM, None, None).await;
    assert_eq!(
        room["value"]["players"],
        serde_json::json!(["Mara Vell", "Danny"])
    );
    assert_eq!(room["value"]["state"], "gathering");

    let log = log_text(&running);
    assert!(log.contains("SEAT join_requested kind=guest"), "{log}");
    assert!(log.contains("SEAT seated seat=1 kind=guest"), "{log}");
    assert!(
        log.contains("SEAT hello known=yes seat=1 seated=yes"),
        "{log}"
    );
    assert!(!log.contains(&phone), "the key is never logged");
    let hash = hex_digits(&Sha256::digest(phone.as_bytes()));
    assert!(!log.contains(&hash), "nor its hash");
}

#[tokio::test]
async fn a_missing_or_malformed_key_and_a_bad_name_are_refused_with_stable_keys() {
    let running = start(&STARTER_FILES).await;
    let (status, missing) = call(&running, "POST", PLAY_HELLO, None, Some("{}")).await;
    assert_eq!(
        (status.as_str(), &missing["error"]["error"]),
        ("400", &Value::from("missing_key"))
    );
    let (_, invalid) = call(&running, "POST", PLAY_HELLO, Some("ABC"), Some("{}")).await;
    assert_eq!(invalid["error"]["error"], "invalid_key");
    let (status, named) = join_as_guest(&running, &key(3), "<script>").await;
    assert_eq!(status, "400");
    assert_eq!(named["error"]["error"], SeatErrorKind::InvalidName.key());
    assert!(log_text(&running).contains("SEAT refused reason=invalid_name"));
}

#[tokio::test]
async fn signing_in_is_refused_accounts_not_available_with_no_fake_sign_in() {
    let running = start(&STARTER_FILES).await;
    let (status, refused) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&key(4)),
        Some(r#"{"kind":"account"}"#),
    )
    .await;
    assert_eq!(status, "501");
    assert_eq!(
        refused["error"]["error"],
        SeatErrorKind::AccountsNotAvailable.key()
    );
    let (_, status_reply) = call(&running, "GET", PLAY_JOIN_STATUS, Some(&key(4)), None).await;
    assert_eq!(status_reply["value"]["outcome"], "not_asked");
}

#[tokio::test]
async fn a_friend_waits_for_the_hosts_approval_and_is_seated_under_their_profile() {
    let running = start(&STARTER_FILES).await;
    let (_, friend) = call(
        &running,
        "POST",
        path::FRIEND_ADD,
        None,
        Some(r#"{"name":"Danny"}"#),
    )
    .await;
    let profile = friend["value"]["id"].as_u64().expect("a profile id");
    let phone = key(5);
    let body = format!(r#"{{"kind":"friend","profile":{profile}}}"#);
    let (_, asked) = call(&running, "POST", PLAY_JOIN, Some(&phone), Some(&body)).await;
    assert_eq!(asked["value"]["outcome"], "awaiting_approval");
    let request = asked["value"]["request"].as_u64().expect("a request");
    let (_, waiting) = call(&running, "GET", PLAY_JOIN_STATUS, Some(&phone), None).await;
    assert_eq!(waiting["value"]["outcome"], "awaiting_approval");
    assert!(
        !is_seated_at_table(&running, 1).await,
        "not before approval"
    );

    let approve = format!(r#"{{"request":{request}}}"#);
    let (status, approved) = call(&running, "POST", path::APPROVE, None, Some(&approve)).await;
    assert_eq!(status, "200", "{approved}");
    let (_, seated) = call(&running, "GET", PLAY_JOIN_STATUS, Some(&phone), None).await;
    assert_eq!(seated["value"]["outcome"], "seated");
    assert_eq!(seated["value"]["seat"]["kind"], "friend");
    assert_eq!(seated["value"]["seat"]["name"], "Danny");
    assert!(is_seated_at_table(&running, 1).await);
    let log = log_text(&running);
    assert!(
        log.contains(&format!("SEAT approved request={request} seat=1")),
        "{log}"
    );
    assert!(log.contains("SEAT seated seat=1 kind=friend"), "{log}");
}

#[tokio::test]
async fn a_refused_friend_is_told_once_and_can_ask_again() {
    let running = start(&STARTER_FILES).await;
    call(
        &running,
        "POST",
        path::FRIEND_ADD,
        None,
        Some(r#"{"name":"Danny"}"#),
    )
    .await;
    let phone = key(6);
    let (_, asked) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&phone),
        Some(r#"{"kind":"friend","profile":1}"#),
    )
    .await;
    let request = asked["value"]["request"].as_u64().expect("a request");
    let refuse = format!(r#"{{"request":{request}}}"#);
    let (status, _) = call(&running, "POST", path::REFUSE, None, Some(&refuse)).await;
    assert_eq!(status, "200");
    let (_, told) = call(&running, "GET", PLAY_JOIN_STATUS, Some(&phone), None).await;
    assert_eq!(told["value"]["outcome"], "refused");
    let (_, after) = call(&running, "GET", PLAY_JOIN_STATUS, Some(&phone), None).await;
    assert_eq!(after["value"]["outcome"], "not_asked");
    assert!(log_text(&running).contains("SEAT refused reason=refused_by_host"));
    let (_, unknown) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&phone),
        Some(r#"{"kind":"friend","profile":99}"#),
    )
    .await;
    assert_eq!(unknown["error"]["error"], "unknown_friend");
}

#[tokio::test]
async fn the_seat_comes_from_the_key_and_a_body_naming_another_seat_moves_nobody() {
    let running = start(&STARTER_FILES).await;
    let first = key(7);
    join_as_guest(&running, &first, "Mara").await;
    join_as_guest(&running, &key(8), "Danny").await;

    let (status, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_INTENT,
        Some(&first),
        Some(r#"{"action":"ready","seat":2}"#),
    )
    .await;
    assert_eq!(status, "400", "a seat in the body is not read: {refused}");
    let (_, ready) = call(
        &running,
        "POST",
        PLAY_SEAT_INTENT,
        Some(&first),
        Some(r#"{"action":"ready"}"#),
    )
    .await;
    assert_eq!(ready["value"]["outcome"], "applied", "{ready}");

    let (_, own) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&first), None).await;
    assert_eq!(own["value"]["ready"], true, "seat 1 is ready: {own}");
    let (_, other) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&key(8)), None).await;
    assert_eq!(other["value"]["ready"], false, "seat 2 is not: {other}");
    assert_eq!(other["value"]["players"][0]["ready"], true);
    assert_eq!(other["value"]["players"][1]["you"], true);
}

#[tokio::test]
async fn a_player_chooses_one_of_their_own_heroes_for_the_seat_and_hello_carries_it() {
    let running = start(&STARTER_FILES).await;
    let phone = key(15);
    join_as_guest(&running, &phone, "Mara").await;
    join_as_guest(&running, &key(16), "Danny").await;
    let hero = r#"{"seat":1,"name":"Brynn Quarry","species":"Dwarf","class":"Fighter"}"#;
    call(&running, "POST", path::HERO_ADD, None, Some(hero)).await;

    let (_, other) = call(
        &running,
        "POST",
        PLAY_HERO,
        Some(&key(16)),
        Some(r#"{"hero":1}"#),
    )
    .await;
    assert_eq!(other["error"]["error"], SeatErrorKind::UnknownHero.key());
    let (status, chosen) = call(
        &running,
        "POST",
        PLAY_HERO,
        Some(&phone),
        Some(r#"{"hero":1}"#),
    )
    .await;
    assert_eq!(status, "200", "{chosen}");
    assert_eq!(chosen["value"]["hero"], 1);
    let (_, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(hello["value"]["seat"]["hero"], 1, "{hello}");
    assert!(log_text(&running).contains("SEAT hero_chosen seat=1 hero=1"));
}

#[tokio::test]
async fn a_seat_book_player_who_has_not_come_back_is_away_and_not_counted_in_the_room() {
    let running = start(&STARTER_FILES).await;
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books()
            .seat(&KeyHash::of(&key(17)), 3, Holder::Guest, "Vix", 1_000);
    }
    let phone = key(18);
    join_as_guest(&running, &phone, "Danny").await;
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    let players = &view["value"]["players"];
    assert_eq!(players[0]["name"], "Danny");
    assert_eq!(players[0]["present"], true);
    assert_eq!(players[1]["name"], "Vix");
    assert_eq!(players[1]["present"], false, "{view}");
    assert_eq!(view["value"]["stage"], "gathering");
    let (_, room) = call(&running, "GET", PLAY_ROOM, None, None).await;
    assert_eq!(room["value"]["players"], serde_json::json!(["Danny"]));
}

#[tokio::test]
async fn leaving_frees_the_seat_at_the_table_and_in_the_book() {
    let running = start(&STARTER_FILES).await;
    let phone = key(9);
    join_as_guest(&running, &phone, "Mara").await;
    let (_, left) = call(
        &running,
        "POST",
        PLAY_SEAT_INTENT,
        Some(&phone),
        Some(r#"{"action":"leave"}"#),
    )
    .await;
    assert_eq!(left["value"]["outcome"], "left", "{left}");
    assert!(!is_seated_at_table(&running, 1).await);
    let (_, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(hello["value"]["outcome"], "unknown");
    assert!(log_text(&running).contains("SEAT left seat=1"));
}

#[tokio::test]
async fn a_sample_hero_is_listed_only_for_its_own_player() {
    let running = start(&STARTER_FILES).await;
    join_as_guest(&running, &key(10), "Mara").await;
    join_as_guest(&running, &key(11), "Danny").await;
    let hero = r#"{"seat":1,"name":"Vix Ashlantern","species":"Elf","class":"Fighter"}"#;
    let (status, added) = call(&running, "POST", path::HERO_ADD, None, Some(hero)).await;
    assert_eq!(status, "200", "{added}");
    let (_, own) = call(&running, "GET", PLAY_HEROES, Some(&key(10)), None).await;
    assert_eq!(own["value"]["heroes"][0]["name"], "Vix Ashlantern");
    let (_, other) = call(&running, "GET", PLAY_HEROES, Some(&key(11)), None).await;
    assert_eq!(other["value"]["heroes"], serde_json::json!([]));
}

#[tokio::test]
async fn the_seat_book_and_the_friends_book_survive_a_host_restart() {
    let directory = tempfile::TempDir::new().expect("a directory");
    let phone = KeyHash::of(&key(12));
    {
        let desk = SeatDesk::new(directory.path().to_path_buf());
        let mut open = desk.open().await.expect("opens");
        open.books().seat(&phone, 4, Holder::Guest, "Mara", 1_000);
        open.books().add_friend("Danny").expect("adds");
        open.save(crate::seat::book::Book::Seats)
            .await
            .expect("saves");
        open.save(crate::seat::book::Book::Friends)
            .await
            .expect("saves");
    }
    let desk = SeatDesk::new(directory.path().to_path_buf());
    let mut open = desk.open().await.expect("reopens");
    let record = open.books().seat_of(&phone).cloned().expect("kept");
    assert_eq!((record.seat, record.name.as_str()), (4, "Mara"));
    assert_eq!(open.books().friend_cards().len(), 1);
}

#[tokio::test]
async fn a_returning_phone_whose_seat_the_table_lost_continues_into_the_same_seat() {
    let running = start(&STARTER_FILES).await;
    let phone = key(13);
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books()
            .seat(&KeyHash::of(&phone), 3, Holder::Guest, "Mara", 1_000);
    }
    let (_, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(hello["value"]["outcome"], "returning", "{hello}");
    let (_, joined) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&phone),
        Some(r#"{"kind":"returning"}"#),
    )
    .await;
    assert_eq!(joined["value"]["seat"]["seat"], 3, "{joined}");
    assert!(is_seated_at_table(&running, 3).await);
    assert!(log_text(&running).contains("SEAT rejoined seat=3"));
}

#[tokio::test]
async fn the_player_listener_serves_the_seat_routes_and_not_the_seat_dev_routes() {
    let running = start(&STARTER_FILES).await;
    let lan = IpAddr::from([192, 168, 1, 31]);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let routes = crate::player::router(
        Arc::clone(&running.state),
        Audience::Players {
            addresses: vec![lan],
            port,
        },
    )
    .into_make_service_with_connect_info::<SocketAddr>();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, routes).await;
    });
    let host = format!("{lan}:{port}");
    let get =
        |path: &str| format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let room = raw_exchange(port, &get(PLAY_ROOM)).await;
    assert!(room.starts_with("HTTP/1.1 200"), "{room}");
    let players = raw_exchange(port, &get(path::PLAYERS)).await;
    assert!(players.starts_with("HTTP/1.1 404"), "{players}");
    let foreign = format!(
        "POST {PLAY_HELLO} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://evil.example\r\n\
         {SEAT_KEY_HEADER}: {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\
         Connection: close\r\n\r\n{{}}",
        key(14)
    );
    let refused = raw_exchange(port, &foreign).await;
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    server.abort();
}
