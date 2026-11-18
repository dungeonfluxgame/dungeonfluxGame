//! Seats for phones over the loopback listener and the player listener: the three kinds of
//! player, the seat taken from the key, the lobby joined as that seat, the books kept on disk.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use df_playground::path::{
    PLAY_HELLO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_CODE, PLAY_JOIN_STATUS, PLAY_ROOM,
    PLAY_SEAT_ACTION, PLAY_SEAT_VIEW, SEAT_KEY_HEADER,
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

/// A seat action's body (`SeatAction`), with a fresh `said` id when it names none.
fn action(body: &str) -> String {
    static SAID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000);
    let mut value: Value = serde_json::from_str(body).expect("an action body");
    if value.get("said").is_none() {
        value["said"] = SAID
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .into();
    }
    value.to_string()
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

/// The room code the table shows now.
fn room_code(running: &Running) -> String {
    running.state.seats().room_code()
}

async fn join_as_guest(running: &Running, seat_key: &str, name: &str) -> (String, Value) {
    let body = serde_json::json!({ "kind": "guest", "name": name, "code": room_code(running) })
        .to_string();
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
        Some(&format!(
            r#"{{"kind":"account","code":"{}"}}"#,
            room_code(&running)
        )),
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
    let body = format!(
        r#"{{"kind":"friend","profile":{profile},"code":"{}"}}"#,
        room_code(&running)
    );
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
        Some(&format!(
            r#"{{"kind":"friend","profile":1,"code":"{}"}}"#,
            room_code(&running)
        )),
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
        Some(&format!(
            r#"{{"kind":"friend","profile":99,"code":"{}"}}"#,
            room_code(&running)
        )),
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
        PLAY_SEAT_ACTION,
        Some(&first),
        Some(&action(r#"{"action":"lobby","step":"ready","seat":2}"#)),
    )
    .await;
    assert_eq!(status, "400", "a seat in the body is not read: {refused}");
    let (_, ready) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&first),
        Some(&action(r#"{"action":"lobby","step":"ready"}"#)),
    )
    .await;
    assert_eq!(ready["value"]["lobby"]["outcome"], "applied", "{ready}");

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
        PLAY_SEAT_ACTION,
        Some(&key(16)),
        Some(&action(r#"{"action":"hero","hero":1}"#)),
    )
    .await;
    assert_eq!(other["error"]["error"], SeatErrorKind::UnknownHero.key());
    let (status, chosen) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"hero","hero":1}"#)),
    )
    .await;
    assert_eq!(status, "200", "{chosen}");
    assert_eq!(chosen["value"]["seat"]["hero"], 1, "{chosen}");
    let (_, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(hello["value"]["seat"]["hero"], 1, "{hello}");
    assert_eq!(
        hello["value"]["seat"]["hero_locked"], false,
        "not locked in yet: the phone goes back to the look studio"
    );
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books()
            .lock_hero(1, Some("hero-portrait.png".to_owned()), None)
            .expect("the hero is in the library");
    }
    let (_, locked) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(locked["value"]["seat"]["hero_locked"], true, "{locked}");
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
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"lobby","step":"leave"}"#)),
    )
    .await;
    assert_eq!(left["value"]["lobby"]["outcome"], "left", "{left}");
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
async fn a_returning_phone_whose_seat_the_table_lost_goes_straight_back_into_it() {
    let running = start(&STARTER_FILES).await;
    let phone = key(13);
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books()
            .seat(&KeyHash::of(&phone), 3, Holder::Guest, "Mara", 1_000);
    }
    // The hello alone seats it again: a phone with a stored key goes straight to its seat.
    let (_, hello) = call(&running, "POST", PLAY_HELLO, Some(&phone), Some("{}")).await;
    assert_eq!(hello["value"]["outcome"], "seated", "{hello}");
    assert_eq!(hello["value"]["seat"]["seat"], 3, "{hello}");
    assert!(is_seated_at_table(&running, 3).await);
    assert!(log_text(&running).contains("SEAT rejoined seat=3"));
    // Continuing by hand still works and keeps the same seat.
    let (_, joined) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&phone),
        Some(r#"{"kind":"returning"}"#),
    )
    .await;
    assert_eq!(joined["value"]["seat"]["seat"], 3, "{joined}");
}

#[tokio::test]
async fn ask_ids_start_from_the_boot_so_a_restart_never_repeats_one() {
    let running = start(&STARTER_FILES).await;
    let first = running.state.seats().talk().first_ask();
    assert!(
        first > 1_000_000_000_000,
        "seeded from the wall clock: {first}"
    );
    let restarted = start(&STARTER_FILES).await;
    assert!(restarted.state.seats().talk().first_ask() >= first);
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
    // Acting as a seat by its number is the loopback listener's alone.
    let as_seat = as_seat_request(&host, 3);
    let dev_refused = raw_exchange(port, &as_seat).await;
    assert!(dev_refused.starts_with("HTTP/1.1 403"), "{dev_refused}");
    server.abort();
}

/// A GET of the seat's view acting as the seat `seat` (the `dev` command line's header).
fn as_seat_request(host: &str, seat: u64) -> String {
    format!(
        "GET {PLAY_SEAT_VIEW} HTTP/1.1\r\nHost: {host}\r\nx-df-dev-seat: {seat}\r\n\
         Connection: close\r\n\r\n"
    )
}

#[tokio::test]
async fn the_dev_command_line_acts_as_a_seat_by_its_number_on_the_loopback_listener() {
    let running = start(&STARTER_FILES).await;
    let phone = key(31);
    let (_, joined) = join_as_guest(&running, &phone, "Mara").await;
    let seat = joined["value"]["seat"]["seat"].as_u64().expect("a seat");
    let (_, by_key) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    let as_seat = as_seat_request("127.0.0.1", seat);
    let answer = raw_exchange(running.port, &as_seat).await;
    let body: Value = answer
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert_eq!(
        body["value"]["players"], by_key["value"]["players"],
        "{answer}"
    );
    let nobody = as_seat_request("127.0.0.1", 99);
    let refused = raw_exchange(running.port, &nobody).await;
    assert!(refused.contains("unknown_seat"), "{refused}");
}

/// Stores the tests' innkeeper persona in the host's playground and seats a guest bound to it.
async fn seat_playing_the_innkeeper(running: &Running, phone: &str, seat: u64) {
    let directory = running.state.playground().files().assets().join("personas");
    crate::persona::write_persona(&directory, &crate::persona::fixtures::innkeeper())
        .expect("stores the persona");
    join_as_guest(running, phone, "Mara").await;
    let (status, bound) = call(
        running,
        "POST",
        path::PERSONA,
        None,
        Some(&format!(r#"{{"seat":{seat},"persona":"innkeeper"}}"#)),
    )
    .await;
    assert_eq!(status, "200", "{bound}");
}

#[tokio::test]
async fn a_seat_says_a_line_only_when_asked_and_the_runner_takes_it_as_written() {
    let running = start(&STARTER_FILES).await;
    let phone = key(20);
    let say = r#"{"action":"answer","said":1,"text":"  I take the road north to the mill.  "}"#;
    join_as_guest(&running, &key(21), "Danny").await;
    let (_, unbound) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&key(21)),
        Some(say),
    )
    .await;
    assert_eq!(unbound["error"]["error"], SeatErrorKind::NoPersona.key());

    seat_playing_the_innkeeper(&running, &phone, 2).await;
    let (_, early) = call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(say)).await;
    assert_eq!(early["error"]["error"], SeatErrorKind::NothingAsked.key());

    let ask = r#"{"to":{"seats":[2]},"prompt":"Where do you go next?",
        "cards":[{"label":"Take the road","line":"I take the road on."}]}"#;
    let (_, asked) = call(&running, "POST", path::ASK, None, Some(ask)).await;
    let ask_id = asked["value"]["ask"].as_u64().expect("an ask id");
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(
        view["value"]["ask"]["prompt"], "Where do you go next?",
        "{view}"
    );
    assert!(
        view["value"]["story"].is_null(),
        "a bound seat shows the lobby until its table starts: {view}"
    );
    for seat_key in [key(21), phone.clone()] {
        let ready = action(r#"{"action":"lobby","step":"ready"}"#);
        call(
            &running,
            "POST",
            PLAY_SEAT_ACTION,
            Some(&seat_key),
            Some(&ready),
        )
        .await;
    }
    let start = action(r#"{"action":"lobby","step":"start"}"#);
    let (_, started) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&start),
    )
    .await;
    assert_eq!(started["value"]["lobby"]["outcome"], "applied", "{started}");
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(view["value"]["story"]["hero"], "Marta Venn", "{view}");

    let (status, said) = call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(say)).await;
    assert_eq!(status, "200", "{said}");
    assert_eq!(said["value"]["ask"], ask_id);
    // Sent again (the answer was lost): answered as before, and said once.
    let (status, again) = call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(say)).await;
    assert_eq!(status, "200", "{again}");
    assert_eq!(again["value"]["ask"], ask_id, "{again}");
    // The same draft id with other words is refused.
    let changed = r#"{"action":"answer","said":1,"text":"I stay at the inn."}"#;
    let (_, refused_changed) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(changed),
    )
    .await;
    assert_eq!(
        refused_changed["error"]["error"], "said_changed",
        "{refused_changed}"
    );
    let (_, taken) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":{"seats":[2]},"wait_ms":1000}"#),
    )
    .await;
    assert_eq!(taken["value"]["outcome"], "said");
    assert_eq!(taken["value"]["text"], "I take the road north to the mill.");
    let (_, once) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":{"seats":[2]},"wait_ms":0}"#),
    )
    .await;
    assert_eq!(
        once["value"]["outcome"], "empty",
        "a repeat is said once: {once}"
    );
    let (_, after) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(after["value"]["ask"], Value::Null, "an answered ask ends");

    // Talk is a turn of its own: it needs no ask, answers none and leaves an open one open.
    let (_, asked) = call(&running, "POST", path::ASK, None, Some(ask)).await;
    let talk = r#"{"action":"talk","said":2,"text":"Good day!"}"#;
    let (status, talked) = call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(talk)).await;
    assert_eq!(status, "200", "{talked}");
    assert_eq!(talked["value"]["ask"], Value::Null);
    let (_, open) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(open["value"]["ask"]["id"], asked["value"]["ask"], "{open}");
    let (_, heard) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":{"seats":[2]},"wait_ms":1000}"#),
    )
    .await;
    assert_eq!(heard["value"]["kind"], "talk", "{heard}");
    assert_eq!(heard["value"]["ask"], Value::Null);

    // A look card's answer carries its thing; a thing no card of the ask is about is refused.
    // A thing listed twice (designed and noticed) is one card.
    let look = r#"{"to":{"seats":[2]},"kind":"look","prompt":"What do you step up to?",
        "cards":[{"label":"The gate","line":"I step up to the gate.","thing":"garden_gate"},
                 {"label":"The old gate","line":"I go to the gate.","thing":"garden_gate"}]}"#;
    call(&running, "POST", path::ASK, None, Some(look)).await;
    let (_, shown) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(
        shown["value"]["ask"]["cards"].as_array().map(Vec::len),
        Some(1),
        "{shown}"
    );
    // An answer to an ask the table replaced is refused, never taken as the new ask's.
    let (_, waiting) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    let open_ask = waiting["value"]["ask"]["id"].as_u64().expect("an open ask");
    let stale = format!(
        r#"{{"action":"answer","said":3,"text":"I step up.","ask":{}}}"#,
        open_ask - 1
    );
    let (_, refused_stale) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&stale),
    )
    .await;
    assert_eq!(
        refused_stale["error"]["error"], "stale_ask",
        "{refused_stale}"
    );
    let wrong = r#"{"action":"answer","said":4,"text":"I step up to the gate.","thing":"well"}"#;
    let (_, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(wrong),
    )
    .await;
    assert_eq!(refused["error"]["error"], "unknown_thing", "{refused}");
    let card =
        r#"{"action":"answer","said":5,"text":"I step up to the gate.","thing":"garden_gate"}"#;
    let (status, _) = call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(card)).await;
    assert_eq!(status, "200");
    let (_, carded) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":{"seats":[2]},"wait_ms":1000}"#),
    )
    .await;
    assert_eq!(carded["value"]["thing"], "garden_gate", "{carded}");
    let log = log_text(&running);
    assert!(
        log.contains(&format!(
            "SEAT said seat=2 kind=answer ask={ask_id} card_thing=false chars=34"
        )),
        "{log}"
    );
    assert!(
        log.contains("SEAT action seat=2 action=answer repeat=yes"),
        "a repeat is answered as before and said once: {log}"
    );
    assert!(
        log.contains("SEAT said seat=2 kind=talk ask=- card_thing=false chars=9"),
        "{log}"
    );
    assert!(!log.contains("the mill"), "the words are never logged");
}

#[tokio::test]
async fn a_persona_is_played_from_one_seat_only() {
    let running = start(&STARTER_FILES).await;
    seat_playing_the_innkeeper(&running, &key(22), 1).await;
    join_as_guest(&running, &key(23), "Danny").await;
    let (_, taken) = call(
        &running,
        "POST",
        path::PERSONA,
        None,
        Some(r#"{"seat":2,"persona":"innkeeper"}"#),
    )
    .await;
    assert_eq!(taken["error"]["error"], "persona_taken");
    let (_, unknown) = call(
        &running,
        "POST",
        path::PERSONA,
        None,
        Some(r#"{"seat":2,"persona":"nobody"}"#),
    )
    .await;
    assert_eq!(unknown["error"]["error"], "unknown_persona");
}

#[tokio::test]
async fn the_inbox_answers_empty_when_no_line_comes_in_time() {
    let running = start(&STARTER_FILES).await;
    let (_, empty) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":{"seats":[1]},"wait_ms":10}"#),
    )
    .await;
    assert_eq!(empty["value"]["outcome"], "empty");
}

#[tokio::test]
async fn a_ready_seat_starts_the_game_through_its_seat_intent_and_an_away_player_does_not_block_it()
{
    let running = start(&STARTER_FILES).await;
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books()
            .seat(&KeyHash::of(&key(30)), 5, Holder::Guest, "Vix", 1_000);
    }
    let phone = key(31);
    join_as_guest(&running, &phone, "Danny").await;
    let (_, before) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(before["value"]["can_start"], true, "{before}");
    call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"lobby","step":"ready"}"#)),
    )
    .await;
    let (_, started) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"lobby","step":"start"}"#)),
    )
    .await;
    assert_eq!(started["value"]["lobby"]["outcome"], "applied", "{started}");
    let (_, after) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(after["value"]["stage"], "playing", "{after}");
}

#[tokio::test]
async fn the_scenario_shell_is_the_one_named_and_a_missing_one_is_refused_with_what_was_tried() {
    use crate::seat::scenario::find_shell;
    let existing = std::env::current_exe().expect("the test binary has a path");
    assert_eq!(find_shell(Some(&existing)).await, Ok(existing));
    let missing = std::path::PathBuf::from("no-such-dir/bash.exe");
    let refused = find_shell(Some(&missing)).await.expect_err("no such shell");
    assert_eq!(refused.tried, [missing]);
    assert!(refused.to_string().contains("--scenario-shell"));
}

// TODO(DEV-12): with a scenario, a seat bound by an earlier host's day shows the lobby
// (`Scenario::playing`); `clear_bindings` at a host start and the day's end unbind it; a runner
// that exits with `PREPARE_FAILED_STATUS` reads as `DayEnd::NotStarted`.
// TODO(DEV-12): a perception's thought reaches the seat's story in its parts; one the model
// reported as the private side (`leak_suspect`) never does.

#[tokio::test]
async fn a_player_turns_talking_by_voice_on_for_their_own_seat_and_off_is_the_start() {
    use df_playground::path::PLAY_SEAT_TRANSCRIBE;
    let running = start(&STARTER_FILES).await;
    let phone = key(40);
    join_as_guest(&running, &phone, "Wren").await;
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(view["value"]["stt"]["on"], false, "{view}");
    // Off: a recording is refused as speech to text turned off, and the refusal is logged.
    // A real recording: a tenth of a second of 16 kHz 16-bit silence, sent as the phone sends it.
    let silence = "\0".repeat(3_200);
    let recording = format!(
        "POST {PLAY_SEAT_TRANSCRIBE} HTTP/1.1\r\nHost: 127.0.0.1\r\n{SEAT_KEY_HEADER}: {phone}\r\n\
         Content-Type: audio/L16;rate=16000\r\nContent-Length: {}\r\nConnection: close\r\n\r\n\
         {silence}",
        silence.len()
    );
    let answer = raw_exchange(running.port, &recording).await;
    let off: Value = answer
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    assert_eq!(off["error"]["error"], "disabled", "{answer}");
    let (status, on) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"voice","on":true}"#)),
    )
    .await;
    assert_eq!(status, "200", "{on}");
    assert_eq!(on["value"]["stt"]["on"], true, "{on}");
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(view["value"]["stt"]["on"], true, "{view}");
    // The seat's switch on is not enough: while the playground's STT switch is off, nothing is
    // spent (`Speech::transcribe` refuses before any supplier call).
    let answer = raw_exchange(running.port, &recording).await;
    let still_off: Value = answer
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    assert_eq!(still_off["error"]["error"], "disabled", "{answer}");
    let log = log_text(&running);
    assert!(log.contains("SEAT stt seat=1 on=true"), "{log}");
    assert_eq!(
        log.matches("SEAT transcribe_refused seat=1 error=disabled")
            .count(),
        2,
        "{log}"
    );
    assert!(!log.contains("SPEECH transcribed"), "{log}");
    // TODO(DEV-12): with a fake speech supplier, an "on" seat's recording is written down and
    // logged as `SEAT transcribed seat= ms= chars=`; the mic on a real phone (browser check).
}

#[tokio::test]
async fn a_lost_key_is_refused_alike_on_the_seat_look_and_pages_routes() {
    let running = start(&STARTER_FILES).await;
    let lost = key(77);
    for (method, route) in [
        ("GET", PLAY_SEAT_VIEW),
        ("GET", df_playground::path::PLAY_LOOK),
        ("GET", df_playground::path::PLAY_SEAT_PAGES),
    ] {
        let (status, refused) = call(&running, method, route, Some(&lost), None).await;
        assert_eq!(status, "404", "{route}: {refused}");
        assert_eq!(
            refused["error"]["error"], "unknown_key",
            "{route}: {refused}"
        );
        assert!(
            refused["error"]["cause"].is_null(),
            "{route}: one shape, not nested: {refused}"
        );
        let kind = refused["error"]["error"]
            .as_str()
            .and_then(SeatErrorKind::from_key);
        assert_eq!(
            kind.map(SeatErrorKind::player_words),
            Some("This phone is not at the table. Join again.")
        );
    }
}

#[test]
fn every_seat_and_look_refusal_is_tagged_with_the_kind_the_phone_reads() {
    use crate::look::request::LookError;
    use crate::seat::request::SeatError;
    use df_playground::dev_reply::ErrorTag;
    use df_playground::look::wire::LookErrorKind;
    let seat_errors = [
        SeatError::UnknownKey,
        SeatError::PersonaTaken { seat: 2 },
        SeatError::UnknownPersona {
            persona: "mara".to_owned(),
        },
        SeatError::UnknownListener {
            to: "mara".to_owned(),
        },
        SeatError::NotApplied,
        SeatError::SaidChanged { said: 4 },
        SeatError::UnknownRequest { request: 9 },
        SeatError::UnknownSeat { seat: 3 },
        SeatError::FriendsFull,
        SeatError::HeroesFull,
    ];
    for error in seat_errors {
        let tagged = serde_json::to_value(&error).expect("a seat error writes");
        assert_eq!(tagged["error"], error.key(), "{tagged}");
        assert_eq!(SeatErrorKind::from_tag(error.key()), Some(error.kind()));
    }
    let look_errors = [
        LookError::NoHero { seat: 1 },
        LookError::KeptChanged { kept: Some(2) },
        LookError::LockBusy,
        LookError::Stopped,
        LookError::Seat(SeatError::UnknownKey),
    ];
    for error in look_errors {
        let tagged = serde_json::to_value(&error).expect("a look error writes");
        assert_eq!(tagged["error"], error.key(), "{tagged}");
        assert_eq!(LookErrorKind::from_tag(error.key()), Some(error.kind()));
    }
    let lost = serde_json::to_value(LookError::Seat(SeatError::UnknownKey)).expect("writes");
    assert_eq!(lost, serde_json::json!({"error": "unknown_key"}));
    assert_eq!(
        serde_json::from_value::<LookError>(lost).ok(),
        Some(LookError::Seat(SeatError::UnknownKey))
    );
}

#[tokio::test]
async fn a_note_from_the_table_shows_on_the_seat_until_its_next_ask() {
    let running = start(&STARTER_FILES).await;
    let phone = key(40);
    seat_playing_the_innkeeper(&running, &phone, 1).await;
    let note = r#"{"seat":1,"text":"The painter is still at work."}"#;
    let (status, _) = call(&running, "POST", path::NOTE, None, Some(note)).await;
    assert_eq!(status, "200");
    let (_, noted) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(
        noted["value"]["notice"], "The painter is still at work.",
        "{noted}"
    );
    let ask = r#"{"to":{"seats":[1]},"prompt":"Where do you go next?","cards":[]}"#;
    call(&running, "POST", path::ASK, None, Some(ask)).await;
    let (_, asked) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(asked["value"]["notice"], Value::Null, "{asked}");
    let log = log_text(&running);
    assert!(log.contains("SEAT noted seat=1 chars=29"), "{log}");
    assert!(!log.contains("still at work"), "the words are never logged");
}

#[tokio::test]
async fn the_bag_uses_and_drops_a_thing_and_shows_the_purse_and_what_things_are_worth() {
    let running = start(&STARTER_FILES).await;
    let phone = key(41);
    seat_playing_the_innkeeper(&running, &phone, 1).await;
    running.write_asset(
        "world/inventory/innkeeper.json",
        &serde_json::json!({"schema": 1, "owner": "innkeeper", "kind": "character", "items": [
            {"id": "coins", "name": "silver pennies", "look": "Small silver coins.",
             "use": "consumed", "count": 20, "coin": "sp"},
            {"id": "rags", "name": "rags", "look": "Clean rags.", "use": "consumed", "count": 3,
             "price_cp": 2},
            {"id": "boards", "name": "boards", "look": "Pine boards.", "use": "consumed",
             "count": 1}
        ]})
        .to_string(),
    );
    let bag = |body: &'static str| {
        let (running, phone) = (&running, phone.clone());
        async move {
            call(
                running,
                "POST",
                PLAY_SEAT_ACTION,
                Some(&phone),
                Some(&action(&format!(r#"{{"action":"bag","bag":{body}}}"#))),
            )
            .await
        }
    };
    let (status, used) = bag(r#"{"act":"use","item":"rags"}"#).await;
    assert_eq!(status, "200", "{used}");
    assert_eq!(
        used["value"]["bag"],
        serde_json::json!({"count": 2, "offered": false})
    );
    let (_, dropped) = bag(r#"{"act":"drop","item":"boards"}"#).await;
    assert_eq!(dropped["value"]["bag"]["count"], 0, "{dropped}");
    let (_, gone) = bag(r#"{"act":"drop","item":"boards"}"#).await;
    assert_eq!(gone["error"]["error"], "not_carried", "{gone}");
    let (_, nobody) = bag(r#"{"act":"give","item":"rags","to":"nobody"}"#).await;
    assert_eq!(nobody["error"]["error"], "unknown_listener", "{nobody}");
    let (_, pages) = call(
        &running,
        "GET",
        df_playground::path::PLAY_SEAT_PAGES,
        Some(&phone),
        None,
    )
    .await;
    assert_eq!(pages["value"]["purse"], "20 sp", "{pages}");
    let worth: Vec<&Value> = pages["value"]["items"]
        .as_array()
        .map(|items| items.iter().map(|item| &item["worth"]).collect())
        .unwrap_or_default();
    assert_eq!(
        worth,
        [&serde_json::json!("20 sp"), &serde_json::json!("2 cp each")]
    );
    let log = log_text(&running);
    assert!(log.contains("SEAT bag seat=1 act=use item=rags"), "{log}");
    assert!(
        log.contains("WORLD inventory_changed owner=innkeeper item=rags"),
        "{log}"
    );
}

#[tokio::test]
async fn a_day_that_ended_puts_the_table_back_in_the_lobby_with_nobody_ready() {
    let running = start(&STARTER_FILES).await;
    let phone = key(42);
    join_as_guest(&running, &key(43), "Danny").await;
    join_as_guest(&running, &phone, "Mara").await;
    for seat_key in [key(43), phone.clone()] {
        call(
            &running,
            "POST",
            PLAY_SEAT_ACTION,
            Some(&seat_key),
            Some(&action(r#"{"action":"lobby","step":"ready"}"#)),
        )
        .await;
    }
    let (_, started) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"lobby","step":"start"}"#)),
    )
    .await;
    assert_eq!(started["value"]["lobby"]["outcome"], "applied", "{started}");
    let (_, playing) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(playing["value"]["stage"], "playing", "{playing}");

    crate::seat::scenario::not_started(&running.state, &[(2, None)], "test").await;
    let (_, back) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    let log = log_text(&running);
    assert_eq!(back["value"]["stage"], "gathering", "{back}\n{log}");
    assert_eq!(back["value"]["ready"], false, "{back}");
    assert_eq!(back["value"]["can_toggle_ready"], true, "{back}");
    assert!(
        log.contains("SCENARIO lobby seat=2 result=gathering"),
        "{log}"
    );
    // A day that ends at a table already gathering leaves it as it is.
    crate::seat::scenario::not_started(&running.state, &[(2, None)], "test").await;
    assert!(log_text(&running).contains("SCENARIO lobby seat=2 result=already"));
}

#[tokio::test]
async fn a_tap_on_roll_with_no_roll_waiting_is_refused_through_the_one_action_route() {
    let running = start(&STARTER_FILES).await;
    let phone = key(44);
    join_as_guest(&running, &phone, "Mara").await;
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&phone), None).await;
    assert_eq!(view["value"]["pending_roll"], Value::Null, "{view}");
    assert_eq!(view["value"]["last_roll"], Value::Null, "{view}");
    let (status, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&action(r#"{"action":"roll","call":"check-1"}"#)),
    )
    .await;
    assert_eq!(status, "409", "{refused}");
    assert_eq!(refused["error"]["error"], "no_pending_roll", "{refused}");
}

#[test]
fn a_sale_is_paid_exactly_from_the_buyers_coins_in_the_fewest_coins_or_not_at_all() {
    use crate::world::inventory::exact_coins;
    use crate::world::{InventoryItem, ItemUse};
    use df_rules::coins::{Denomination, Price};
    let coin = |id: &str, count: u64, denomination: Denomination| InventoryItem {
        id: id.to_owned(),
        name: id.to_owned(),
        look: id.to_owned(),
        usage: ItemUse::Consumed,
        count,
        price_cp: None,
        coin: Some(denomination),
    };
    let purse = [
        coin("copper", 4, Denomination::Cp),
        coin("gold", 2, Denomination::Gp),
        coin("silver", 9, Denomination::Sp),
    ];
    let paid = exact_coins("buyer", &purse, Price::from_copper(152))
        .expect("counts fit")
        .expect("1 gp 5 sp 2 cp");
    let mut paid: Vec<(String, u64)> = paid
        .into_iter()
        .map(|each| (each.item, each.count))
        .collect();
    paid.sort();
    assert_eq!(
        paid,
        [
            ("copper".to_owned(), 2),
            ("gold".to_owned(), 1),
            ("silver".to_owned(), 5)
        ]
    );
    // A 50 cp painting is never charged a whole gold piece: without exact coins it is refused.
    let gold_only = [coin("gold", 1, Denomination::Gp)];
    assert!(
        exact_coins("buyer", &gold_only, Price::from_copper(50))
            .expect("counts fit")
            .is_none()
    );
}

#[tokio::test]
async fn a_new_phone_needs_the_room_code_and_a_wrong_one_is_refused_and_slowed() {
    let running = start(&STARTER_FILES).await;
    let code = room_code(&running);
    assert!(
        df_playground::lobby::model::room_code(&code).as_deref() == Some(code.as_str()),
        "the code is six of the code's characters: {code}"
    );
    let wrong = if code == "AAAAAA" { "BBBBBB" } else { "AAAAAA" };
    let phone = key(50);
    let body = format!(r#"{{"kind":"guest","name":"Mara","code":"{wrong}"}}"#);
    let (status, refused) = call(&running, "POST", PLAY_JOIN, Some(&phone), Some(&body)).await;
    assert_eq!(status, "403", "{refused}");
    assert_eq!(refused["error"]["error"], SeatErrorKind::CodeMismatch.key());
    assert!(!is_seated_at_table(&running, 1).await);
    let log = log_text(&running);
    assert!(
        log.contains("SEAT join refused reason=code_mismatch misses=1 delay_ms=250"),
        "{log}"
    );
    assert!(!log.contains(&code), "the code is never logged: {log}");
    // Typed in small letters, the right code seats the guest once the miss's wait is over.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let body = format!(
        r#"{{"kind":"guest","name":"Mara","code":"{}"}}"#,
        code.to_lowercase()
    );
    let (status, seated) = call(&running, "POST", PLAY_JOIN, Some(&phone), Some(&body)).await;
    assert_eq!(status, "200", "{seated}");
    assert!(is_seated_at_table(&running, 1).await);
    // The phone's key comes back to its seat without a code.
    let (_, back) = call(
        &running,
        "POST",
        PLAY_JOIN,
        Some(&phone),
        Some(r#"{"kind":"returning"}"#),
    )
    .await;
    assert_eq!(back["value"]["seat"]["seat"], 1, "{back}");
    // `players` lists the code for the developer.
    let (_, players) = call(&running, "GET", path::PLAYERS, None, None).await;
    assert_eq!(players["value"]["room_code"], code.as_str());
}

/// The misses so far and the wait a wrong code opened; a panic for anything else.
fn wrong_code(checked: Result<(), crate::seat::room::Missed>) -> (u32, u128) {
    match checked {
        Err(crate::seat::room::Missed::Wrong { misses, delay }) => (misses, delay.as_millis()),
        other => panic!("not a wrong code: {other:?}"),
    }
}

#[test]
fn misses_from_one_address_wait_longer_up_to_a_bound_and_a_match_forgets_them() {
    use crate::seat::room::{Missed, RoomDoor};
    let mut door = RoomDoor::new();
    let code = door.code().to_owned();
    // Each try comes just as the wait the last miss opened ends.
    let mut at = 0_u64;
    let mut delays = Vec::new();
    for _ in 0..7 {
        let (_, delay) = wrong_code(door.check("10.0.0.2", "------", at));
        delays.push(delay);
        at += u64::try_from(delay).expect("a short wait");
    }
    assert_eq!(delays, vec![250, 500, 1_000, 2_000, 4_000, 4_000, 4_000]);
    // Inside the wait any code, the room's too, is refused unchecked with the time left.
    assert_eq!(
        door.check("10.0.0.2", &code, at - 1_000),
        Err(Missed::TooSoon {
            retry_after: std::time::Duration::from_millis(1_000)
        })
    );
    // Another address starts its own run.
    assert_eq!(wrong_code(door.check("10.0.0.3", "------", at)).0, 1);
    assert!(door.check("10.0.0.2", &code, at).is_ok());
    assert_eq!(wrong_code(door.check("10.0.0.2", "------", at + 1)).0, 1);
    // A run is forgotten after ten quiet minutes.
    assert_eq!(
        wrong_code(door.check("10.0.0.3", "------", at + 10 * 60 * 1_000)).0,
        1
    );
    door.renew();
    assert!(door.check("10.0.0.2", &code, at + 2).is_err() || door.code() == code);
}

#[tokio::test]
async fn parallel_wrong_codes_from_one_address_are_refused_at_once_and_a_right_one_waits_its_turn()
{
    let running = start(&STARTER_FILES).await;
    let code = room_code(&running);
    let wrong = if code == "AAAAAA" { "BBBBBB" } else { "AAAAAA" };
    let body = format!(r#"{{"code":"{wrong}"}}"#);
    let started = std::time::Instant::now();
    let guesses = (0..8_u8).map(|phone| {
        let request = request("POST", PLAY_JOIN_CODE, Some(&key(80 + phone)), Some(&body));
        let port = running.port;
        tokio::spawn(async move { raw_exchange(port, &request).await })
    });
    let mut statuses = Vec::new();
    for guess in guesses.collect::<Vec<_>>() {
        let answer = guess.await.expect("a guess is answered");
        statuses.push(answer.split(' ').nth(1).unwrap_or("").to_owned());
        if statuses.last().map(String::as_str) == Some("429") {
            assert!(answer.contains("too_many_guesses"), "{answer}");
            assert!(answer.to_lowercase().contains("retry-after: 1"), "{answer}");
        }
    }
    // One guess is checked; every other comes inside its wait and is refused without a wait of
    // its own, so no answer is held open.
    statuses.sort();
    assert_eq!(
        statuses.iter().filter(|status| *status == "403").count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|status| *status == "429").count(),
        7,
        "{statuses:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1_500),
        "{:?}",
        started.elapsed()
    );
    // The right code inside the wait is refused too; after it, it is taken.
    let right = format!(r#"{{"code":"{code}"}}"#);
    let (status, early) = call(
        &running,
        "POST",
        PLAY_JOIN_CODE,
        Some(&key(90)),
        Some(&right),
    )
    .await;
    assert_eq!(status, "429", "{early}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (status, taken) = call(
        &running,
        "POST",
        PLAY_JOIN_CODE,
        Some(&key(90)),
        Some(&right),
    )
    .await;
    assert_eq!(status, "200", "{taken}");
    let log = log_text(&running);
    assert!(
        log.contains("SEAT code refused reason=too_many_guesses"),
        "{log}"
    );
    assert!(log.contains("SEAT code taken"), "{log}");
    assert!(!log.contains(&code), "the code is never logged: {log}");
}

#[test]
fn the_qr_code_of_a_join_link_is_a_square_of_modules() {
    let qr = crate::seat::room::qr_modules("http://192.168.1.5:7880/play?code=ABC234")
        .expect("a link fits a QR code");
    assert!(qr.is_valid(), "{qr:?}");
    assert!(qr.width >= 21, "{}", qr.width);
}

/// Stores a second persona for the tests, `smith`, and binds `seat` to it.
async fn seat_playing_the_smith(running: &Running, phone: &str, seat: u64) {
    let directory = running.state.playground().files().assets().join("personas");
    let smith: df_content::persona::Persona = serde_json::from_value(serde_json::json!({
        "schema": 1, "id": "smith", "display_name": "Oda Ferrous", "job": "Smith of the village",
        "personality": ["blunt"], "fears": ["fire gone out"],
        "backstory": "Shoes the village's horses.", "private_backstory": "Owes the guild."
    }))
    .expect("a persona");
    crate::persona::write_persona(&directory, &smith).expect("stores the persona");
    join_as_guest(running, phone, "Danny").await;
    let (status, bound) = call(
        running,
        "POST",
        path::PERSONA,
        None,
        Some(&format!(r#"{{"seat":{seat},"persona":"smith"}}"#)),
    )
    .await;
    assert_eq!(status, "200", "{bound}");
}

#[tokio::test]
async fn a_party_ask_is_answered_once_and_the_other_seat_is_told_who_answered() {
    let running = start(&STARTER_FILES).await;
    let (first, second) = (key(60), key(61));
    seat_playing_the_innkeeper(&running, &first, 1).await;
    seat_playing_the_smith(&running, &second, 2).await;
    let ask = r#"{"to":"party","prompt":"Which road?",
        "cards":[{"label":"North","line":"We go north."}]}"#;
    let (status, asked) = call(&running, "POST", path::ASK, None, Some(ask)).await;
    assert_eq!(status, "200", "{asked}");
    assert_eq!(asked["value"]["seats"], serde_json::json!([1, 2]));
    let ask_id = asked["value"]["ask"].as_u64().expect("an ask id");
    for phone in [&first, &second] {
        let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(phone), None).await;
        assert_eq!(view["value"]["ask"]["id"], ask_id, "{view}");
    }
    let answer = action(&format!(
        r#"{{"action":"answer","ask":{ask_id},"text":"We go north."}}"#
    ));
    let (status, said) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&second),
        Some(&answer),
    )
    .await;
    assert_eq!(status, "200", "{said}");
    let (_, told) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&first), None).await;
    assert_eq!(told["value"]["ask"], Value::Null, "{told}");
    assert_eq!(told["value"]["answered_by"]["ask"], ask_id, "{told}");
    assert_eq!(told["value"]["answered_by"]["seat"], 2, "{told}");
    assert_eq!(told["value"]["answered_by"]["hero"], "Danny", "{told}");
    assert_eq!(
        told["value"]["answered_by"]["said"], "We go north.",
        "{told}"
    );
    // The other seat's answer to the closed ask is refused.
    let late = action(&format!(
        r#"{{"action":"answer","ask":{ask_id},"text":"South!"}}"#
    ));
    let (_, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&first),
        Some(&late),
    )
    .await;
    assert_eq!(refused["error"]["error"], "stale_ask", "{refused}");
    // The party's inbox gives the line with the seat that said it.
    let (_, taken) = call(
        &running,
        "POST",
        path::INBOX,
        None,
        Some(r#"{"from":"party","wait_ms":1000}"#),
    )
    .await;
    assert_eq!(taken["value"]["seat"], 2, "{taken}");
    assert_eq!(taken["value"]["text"], "We go north.", "{taken}");
    // The next ask of the seat clears the notice.
    let next = r#"{"to":{"seats":[1]},"prompt":"And now?","cards":[]}"#;
    call(&running, "POST", path::ASK, None, Some(next)).await;
    let (_, cleared) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&first), None).await;
    assert_eq!(cleared["value"]["answered_by"], Value::Null, "{cleared}");
}

#[test]
fn players_ask_and_inbox_name_a_seat_some_seats_or_the_party() {
    use crate::seat::cli::{PlayersCommand, parse};
    use crate::seat::request::SeatsNamed;
    let words = |line: &str| -> Vec<String> { line.split(' ').map(str::to_owned).collect() };
    let Ok(PlayersCommand::Ask(ask)) = parse(&words("ask party prompt=Which")) else {
        panic!("a party ask");
    };
    assert_eq!(ask.to, SeatsNamed::Party);
    let Ok(PlayersCommand::Ask(ask)) = parse(&words("ask 1,2 prompt=Which")) else {
        panic!("an ask of two seats");
    };
    assert_eq!(ask.to, SeatsNamed::Seats(vec![1, 2]));
    let Ok(PlayersCommand::Inbox(wait)) = parse(&words("inbox party wait=5")) else {
        panic!("the party's inbox");
    };
    assert_eq!((wait.from, wait.wait_ms), (SeatsNamed::Party, 5_000));
    assert!(matches!(
        parse(&words("ask 3 clear")),
        Ok(PlayersCommand::AskClear(SeatsNamed::Seats(seats))) if seats == vec![3]
    ));
    assert!(parse(&words("ask everyone prompt=Which")).is_err());
}

#[tokio::test]
async fn a_clue_not_on_the_board_is_refused_and_closing_with_none_shown_is_refused() {
    let running = start(&STARTER_FILES).await;
    let phone = key(70);
    join_as_guest(&running, &phone, "Mara").await;
    let show = action(r#"{"action":"show_clue","clue":"no-such-clue"}"#);
    let (_, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&show),
    )
    .await;
    let close = action(r#"{"action":"close_clue"}"#);
    let (_, none) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&close),
    )
    .await;
    // Without a scene on the screen the table takes neither.
    let kinds = [
        refused["error"]["error"].clone(),
        none["error"]["error"].clone(),
    ];
    assert!(
        kinds == [Value::from("unknown_clue"), Value::from("no_clue_shown")]
            || kinds == [Value::from("not_applied"), Value::from("not_applied")],
        "{refused} {none}"
    );
    // TODO(DEV-12): a clue on the board shows on the TV (`shown_clue`), closes, and a new plate
    // clears it, against a fixture with a clue board.
}

#[tokio::test]
async fn a_cast_without_a_built_hero_is_refused_and_moves_nothing() {
    let running = start(&STARTER_FILES).await;
    let phone = key(71);
    join_as_guest(&running, &phone, "Mara").await;
    let cast = action(r#"{"action":"cast","spell":"detect-magic","level":1,"as_ritual":true}"#);
    let (_, refused) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&phone),
        Some(&cast),
    )
    .await;
    assert_eq!(
        refused["error"]["error"],
        SeatErrorKind::NoHeroToPlay.key(),
        "{refused}"
    );
    // TODO(DEV-12): a built wizard's cast spends the slot and puts `kind=cast` in the inbox; a buy
    // moves coins and goods once, and a repeat of its `said` answers the stored outcome.
}

#[tokio::test]
async fn a_buy_from_a_shop_that_is_not_here_is_refused_not_here_and_moves_nothing() {
    let running = start(&STARTER_FILES).await;
    let phone = key(72);
    seat_playing_the_innkeeper(&running, &phone, 1).await;
    let buy = action(r#"{"action":"buy","shop":"old-town-smith","item":"dagger","qty":1}"#);
    let (status, refused) =
        call(&running, "POST", PLAY_SEAT_ACTION, Some(&phone), Some(&buy)).await;
    assert_eq!(status, "404", "{refused}");
    assert_eq!(
        refused["error"]["error"],
        SeatErrorKind::NotHere.key(),
        "{refused}"
    );
    assert_eq!(
        SeatErrorKind::NotHere.player_words(),
        "That shop isn't here."
    );
    let log = log_text(&running);
    assert!(log.contains("SEAT refused reason=not_here"), "{log}");
    assert!(!log.contains("INVENTORY buy"), "{log}");
}

#[test]
fn a_seat_is_away_after_two_minutes_unseen_and_seen_is_written_at_most_every_five_seconds() {
    use crate::seat::book::Books;
    use crate::seat::presence::{AWAY_AFTER_MS, Presence, SEEN_EVERY_MS, is_away};
    assert!(!is_away(10_000, 10_000 + AWAY_AFTER_MS - 1));
    assert!(is_away(10_000, 10_000 + AWAY_AFTER_MS));
    let mut books = Books::default();
    let phone = KeyHash::of(&key(80));
    books.seat(&phone, 1, Holder::Guest, "Wren", 10_000);
    assert!(!books.seen(&phone, 10_000 + SEEN_EVERY_MS - 1), "too soon");
    assert!(books.seen(&phone, 10_000 + SEEN_EVERY_MS));
    assert_eq!(books.records()[0].last_seen_ms, 10_000 + SEEN_EVERY_MS);
    assert!(!books.seen(&KeyHash::of(&key(81)), 99_000), "no seat");
    // Each seat goes away once and comes back once.
    let mut presence = Presence::default();
    let start = 1_000_000;
    assert_eq!(
        presence.update(&[(1, start), (2, start)], start + 1),
        (vec![], vec![])
    );
    let later = start + AWAY_AFTER_MS;
    assert_eq!(
        presence.update(&[(1, later), (2, start)], later),
        (vec![2], vec![])
    );
    assert_eq!(
        presence.update(&[(1, later), (2, start)], later + 1),
        (vec![], vec![])
    );
    assert_eq!(
        presence.update(&[(1, later), (2, later + 2)], later + 2),
        (vec![], vec![2])
    );
}

#[tokio::test]
async fn an_away_seat_leaves_the_lobby_so_it_neither_blocks_the_start_nor_plays_and_comes_back() {
    let running = start(&STARTER_FILES).await;
    let (here, gone) = (key(82), key(83));
    join_as_guest(&running, &here, "Mara").await;
    join_as_guest(&running, &gone, "Wren").await;
    assert!(is_seated_at_table(&running, 2).await);
    // The second phone was closed long ago: last seen at the start of time.
    {
        let mut open = running.state.seats().open().await.expect("opens");
        open.books().touch(&KeyHash::of(&gone), 1_000);
    }
    let (_, view) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&here), None).await;
    let players = view["value"]["players"]
        .as_array()
        .expect("players")
        .clone();
    assert_eq!(players[1]["name"], "Wren", "{view}");
    assert_eq!(players[1]["present"], false, "listed away: {view}");
    assert!(!is_seated_at_table(&running, 2).await, "out of the lobby");
    let log = log_text(&running);
    assert_eq!(log.matches("SEAT away seat=2").count(), 1, "{log}");
    call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&here),
        Some(&action(r#"{"action":"lobby","step":"ready"}"#)),
    )
    .await;
    let (_, started) = call(
        &running,
        "POST",
        PLAY_SEAT_ACTION,
        Some(&here),
        Some(&action(r#"{"action":"lobby","step":"start"}"#)),
    )
    .await;
    assert_eq!(started["value"]["lobby"]["outcome"], "applied", "{started}");
    // The seat stays in the book; its phone calling again is back.
    let (status, _) = call(&running, "GET", PLAY_SEAT_VIEW, Some(&gone), None).await;
    assert_eq!(status, "200");
    let log = log_text(&running);
    assert!(log.contains("SEAT back seat=2"), "{log}");
    assert_eq!(log.matches("SEAT away seat=2").count(), 1, "{log}");
    // TODO(DEV-12): with a scenario, DF_RUN_PLAYERS leaves the away seat out (needs a runner).
}

#[test]
fn the_tv_lobby_shows_each_seat_from_choosing_to_ready_and_away_with_nothing_private() {
    use crate::seat::lobby::{LobbyLook, SeatFacts, lobby_screen};
    use df_playground::lobby::model::{JoinCode, SeatState};
    use df_playground::look::wire::{HeroPack, PackStatus};
    use df_playground::seat::wire::{TableHero, TablePlayer};
    let hero = |name: &str| TableHero {
        id: 7,
        name: name.to_owned(),
        species: "human".to_owned(),
        class: "Wizard".to_owned(),
        portrait: Some("portrait-7.png".to_owned()),
        pack: HeroPack {
            headshot: PackStatus::Ready {
                picture: "headshot-7.png".to_owned(),
            },
            ..HeroPack::default()
        },
    };
    let player = |name: &str, present: bool, ready: bool, hero: Option<TableHero>| TablePlayer {
        name: name.to_owned(),
        present,
        ready,
        you: false,
        hero,
    };
    let seats = vec![
        SeatFacts {
            seat: 1,
            player: player("Mara", true, false, None),
            locked: false,
        },
        SeatFacts {
            seat: 2,
            player: player("Danny", true, false, Some(hero("Odile"))),
            locked: false,
        },
        SeatFacts {
            seat: 3,
            player: player("Wren", true, true, Some(hero("Edda Vane"))),
            locked: true,
        },
        SeatFacts {
            seat: 5,
            player: player("Wren", false, false, None),
            locked: false,
        },
    ];
    let look = LobbyLook {
        title: Some("Town to Tomb".to_owned()),
        picture: None,
    };
    let join = JoinCode {
        game_code: Some("ABC234".to_owned()),
        ..JoinCode::default()
    };
    let screen = lobby_screen(&look, join, &seats);
    assert_eq!(screen.title, "Town to Tomb");
    assert_eq!(screen.seats.len(), 6);
    assert_eq!(screen.begin, None, "the phones start the game");
    let card = |index: usize| &screen.seats[index];
    assert_eq!(card(0).state, SeatState::Creating);
    assert_eq!(card(0).stage.as_deref(), Some("Choosing a hero"));
    assert_eq!(card(0).name.as_deref(), Some("Mara"));
    // A hero chosen but not locked in shows the player, never the hero's name yet.
    assert_eq!(card(1).stage.as_deref(), Some("Making their hero…"));
    assert_eq!(card(1).name.as_deref(), Some("Danny"));
    assert_eq!(card(2).state, SeatState::LockedIn);
    assert_eq!(card(2).name.as_deref(), Some("Edda Vane"));
    assert_eq!(card(2).portrait.as_deref(), Some("headshot-7.png"));
    assert_eq!(card(2).badge, None, "ready shows the state's own word");
    assert_eq!(card(3).state, SeatState::Open);
    assert_eq!(card(4).badge.as_deref(), Some("Away"));
    let party = screen.party.expect("a party count");
    assert_eq!((party.joined, party.capacity), (3, 6));
    assert_eq!(party.detail.as_deref(), Some("1 ready"));
    assert_eq!(
        screen.join.and_then(|join| join.game_code).as_deref(),
        Some("ABC234")
    );
}

#[tokio::test]
async fn the_tv_lobby_follows_the_seats_while_the_table_gathers() {
    let running = start(&STARTER_FILES).await;
    let (first, second) = (key(90), key(91));
    join_as_guest(&running, &first, "Mara").await;
    join_as_guest(&running, &second, "Danny").await;
    call(&running, "GET", PLAY_SEAT_VIEW, Some(&first), None).await;
    let log = log_text(&running);
    assert!(log.contains("SEAT lobby shown seats=2 tv="), "{log}");
    if log.contains("SEAT lobby shown seats=2 tv=shown") {
        let lobby = match running.state.playground().snapshot().scene.lobby {
            df_playground::overlay::Edit::Set(lobby) => lobby,
            other => panic!("no lobby: {other:?}"),
        };
        assert_eq!(lobby.seats[1].name.as_deref(), Some("Danny"));
        // Nothing changed: the lobby is not put up again.
        call(&running, "GET", PLAY_SEAT_VIEW, Some(&second), None).await;
        assert_eq!(log_text(&running).matches("SEAT lobby shown").count(), 1);
    }
    // TODO(DEV-12): `lobby::open` on a host with the clean fixture: the fixture switch is not
    // saved, the backdrop is the hall plate, and Start clears the lobby.
}

#[tokio::test]
async fn removing_a_seat_redraws_the_tv_lobby_without_it() {
    let running = start(&STARTER_FILES).await;
    let (first, second) = (key(92), key(93));
    join_as_guest(&running, &first, "Mara").await;
    join_as_guest(&running, &second, "Danny").await;
    call(&running, "GET", PLAY_SEAT_VIEW, Some(&first), None).await;
    let (status, removed) = call(
        &running,
        "POST",
        path::SEAT_REMOVE,
        None,
        Some(r#"{"seat":2}"#),
    )
    .await;
    assert_eq!(status, "200", "{removed}");
    let log = log_text(&running);
    let left = log.find("SEAT left seat=2").expect("the seat left");
    assert!(
        log[left..].contains("SEAT lobby shown seats=1 tv="),
        "the lobby was not redrawn after the removal: {log}"
    );
}

#[tokio::test]
async fn a_fresh_boot_with_no_day_opens_the_clean_lobby_once_the_engine_runs() {
    let running = start(&STARTER_FILES).await;
    assert!(crate::seat::lobby::engine_started(&running.state).await);
    crate::seat::room::open_lobby(&running.state).await;
    let log = log_text(&running);
    assert!(!log.contains("tv=playing"), "no day runs: {log}");
    assert!(!log.contains("tv=no_engine"), "{log}");
    assert!(log.contains("SEAT code made chars=6"), "{log}");
    // The clean fixture is asked for (the test playground may not have it), never saved.
    assert!(
        log.contains("PLAYGROUND fixture shown name=painters_day by=host persisted=no")
            || log.contains("SEAT lobby fixture_refused"),
        "{log}"
    );
    assert!(log.contains("SEAT lobby backdrop tv="), "{log}");
    // TODO(DEV-12): the boot itself (`serve`), where the opening waits for the engine the server
    // starts as it begins to serve.
}

#[tokio::test]
async fn an_away_seat_bound_to_a_half_made_hero_never_joins_the_party_a_start_begins() {
    let running = start(&STARTER_FILES).await;
    let (first, gone, third) = (key(100), key(101), key(102));
    join_as_guest(&running, &first, "Kazrin").await;
    join_as_guest(&running, &gone, "Wren").await;
    join_as_guest(&running, &third, "Maud").await;
    {
        let mut open = running.state.seats().open().await.expect("opens");
        let books = open.books();
        // A closed tab: last seen long ago, still bound to its half-made hero's day persona.
        books.touch(&KeyHash::of(&gone), 1_000);
        let hero = books
            .add_hero(2, "Odile", "human", "wizard")
            .expect("a hero");
        books
            .choose_hero(&KeyHash::of(&gone), hero.id)
            .expect("chosen");
        books
            .bind_persona(2, Some(&crate::seat::scenario::persona_of(hero.id)))
            .expect("bound");
    }
    let now = df_table::ports::Clock::now_ms(&crate::observe::WallClock);
    crate::seat::presence::sweep(&running.state, now)
        .await
        .expect("sweeps");
    let party = crate::seat::routes::start_party(&running.state, now)
        .await
        .expect("a party");
    // The runner's DF_RUN_PLAYERS is written from the party's seats.
    assert_eq!(
        crate::seat::scenario::seats_text(party.iter().map(|record| record.seat)),
        "1,3"
    );
    // TODO(DEV-12): the same through a Start with a scenario and a runner, reading
    // DF_RUN_PLAYERS="1,3" from the runner's environment.
}

/// A dev seat call as a runner with `token` makes it (`x-df-dev-run`), or without one.
fn runner_request(path: &str, token: Option<&str>, body: &str) -> String {
    let token_line = token.map_or_else(String::new, |token| format!("x-df-dev-run: {token}\r\n"));
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{token_line}Content-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn runner_call(running: &Running, path: &str, token: Option<&str>, body: &str) -> Value {
    let answer = raw_exchange(running.port, &runner_request(path, token, body)).await;
    answer
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_runner_left_behind_by_an_earlier_host_is_refused_not_this_run() {
    let running = start(&STARTER_FILES).await;
    join_as_guest(&running, &key(110), "Kazrin").await;
    join_as_guest(&running, &key(111), "Wren").await;
    let inbox = r#"{"from":{"seats":[1]},"wait_ms":0}"#;
    // No day runs: any run token is an old runner's.
    let stale = runner_call(&running, path::INBOX, Some("0ld0ld0ld0ld"), inbox).await;
    assert_eq!(stale["error"]["error"], "not_this_run", "{stale}");
    // A developer's own call (no token) still works while no day runs.
    let own = runner_call(&running, path::INBOX, None, inbox).await;
    assert_eq!(own["value"]["outcome"], "empty", "{own}");

    running
        .state
        .seats()
        .set_scenario(crate::seat::scenario::Scenario::new(
            std::path::PathBuf::from("town.beats"),
            std::path::PathBuf::from("bash"),
            running.port,
        ));
    let scenario = running.state.seats().scenario().expect("a scenario");
    scenario
        .run_for_test(vec![(1, "hero-1".to_owned())], "c0ffeec0ffeec0ffee")
        .await;
    let ask = r#"{"to":{"seats":[1]},"prompt":"Which road?","cards":[]}"#;
    for token in [None, Some("0ld0ld0ld0ld")] {
        let refused = runner_call(&running, path::ASK, token, ask).await;
        assert_eq!(refused["error"]["error"], "not_this_run", "{refused}");
    }
    let asked = runner_call(&running, path::ASK, Some("c0ffeec0ffeec0ffee"), ask).await;
    assert!(asked["value"]["ask"].is_u64(), "{asked}");
    // A seat out of the day is the developer's to ask.
    let other = r#"{"to":{"seats":[2]},"prompt":"And you?","cards":[]}"#;
    let free = runner_call(&running, path::ASK, None, other).await;
    assert!(free["value"]["ask"].is_u64(), "{free}");
    let note = r#"{"seat":1,"text":"Hold on."}"#;
    let noted = runner_call(&running, path::NOTE, None, note).await;
    assert_eq!(noted["error"]["error"], "not_this_run", "{noted}");
    let log = log_text(&running);
    assert!(
        log.contains("SCENARIO stale_runner route=inbox token=0ld0ld0l seats=1"),
        "{log}"
    );
    assert!(
        log.contains("SCENARIO stale_runner route=ask token=- seats=1"),
        "{log}"
    );
}
