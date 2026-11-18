//! The `players` verbs of the dev command line: the seat book, the friends book, the requests a
//! friend's phone made, sample heroes, the character a seat plays, and what the table asks a seat
//! and the lines it says back (for a runner). They call the seat dev routes (`routes::path`).

use df_playground::seat::wire::{AskKind, FriendCard, SayKind, SuggestedLine};

use super::key::DEV_SEAT_HEADER;
use super::request::{
    ApprovalAnswer, Approved, AskClear, AskSet, Asked, FriendAdd, FriendRemove, HeroAdd, HeroAdded,
    Inbox, InboxWait, PersonaBind, PlayersReport, SeatActionReply, SeatNote, SeatRemove, SeatReply,
    SeatsNamed,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};
use crate::observe::WallClock;
use crate::persona::options::take_options;
use df_playground::seat::wire::SeatAction;
use df_table::ports::Clock;

/// Every `players` verb, for the usage refusal and the help.
pub const PLAYERS_USAGE: &str = concat!(
    "players [list] | players friend add <name> | players friend remove <id> | ",
    "players friend list | players pending | players approve <request> | ",
    "players refuse <request> | players hero add <seat> <species> <class> <name> | ",
    "players remove <seat> | players persona <seat> <persona id>|none | ",
    "players ask <seats> [kind=line|move|look] prompt=<text> [note=<text>] ",
    "[card=<label>|<line>[|<thing id>]]... | players ask <seats> clear | ",
    "players note <seat> <text...> | players roll <seat> <roll id> | ",
    "players inbox <seats> [wait=<seconds>] (<seats>: a seat, seats joined by commas as 1,2, ",
    "or party)"
);
/// `players ask`: what the table asks a seat, some seats or the party now.
pub const PLAYERS_ASK_USAGE: &str = concat!(
    "players ask <seats> [kind=line|move|look] prompt=<text> [note=<text>] ",
    "[card=<label>|<line>[|<thing id>]]... | players ask <seats> clear ",
    "(<seats>: a seat, seats joined by commas as 1,2, or party)"
);

/// A `players` command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayersCommand {
    /// The seats, the friend profiles and the requests waiting.
    List,
    FriendAdd(String),
    FriendRemove(u64),
    FriendList,
    Pending,
    Approve(u64),
    Refuse(u64),
    /// A sample hero for the player at a seat, until creation writes real ones.
    HeroAdd(HeroAdd),
    /// The seat leaves the table and the book.
    Remove(u64),
    /// The character the hero at a seat plays, or none.
    Persona(PersonaBind),
    /// What the table asks a seat, some seats or the party now.
    Ask(AskSet),
    /// The table stops asking the seats.
    AskClear(SeatsNamed),
    /// A word from the table for a seat, until its next ask.
    Note(SeatNote),
    /// The seat taps Roll on the roll called for it, through the seat action route as that seat.
    Roll {
        seat: u64,
        call: String,
    },
    /// The next line one of the seats says, waiting up to the milliseconds given.
    Inbox(InboxWait),
}

/// `<seats>`: `party`, a seat number, or seat numbers joined by commas (`1,2`).
fn seats_named(text: &str) -> Result<SeatsNamed, CommandError> {
    if text == "party" {
        return Ok(SeatsNamed::Party);
    }
    text.split(',')
        .map(|seat| {
            seat.parse::<u64>().map_err(|_| {
                bad_choice(
                    "seats",
                    text,
                    "a seat number, seat numbers joined by commas, or party",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(SeatsNamed::Seats)
}

/// `players ask <seats> ...`: the prompt, the note and the cards (`<label>|<line>`), in any order.
fn ask(to: SeatsNamed, words: &[String]) -> Result<AskSet, CommandError> {
    let usage = || CommandError::Usage(PLAYERS_ASK_USAGE);
    let (given, rest) = take_options(words, &["kind", "prompt", "note", "card"]);
    if !rest.is_empty() {
        return Err(usage());
    }
    let mut request = AskSet {
        to,
        kind: AskKind::Line,
        prompt: String::new(),
        cards: Vec::new(),
        note: None,
    };
    for (key, value) in given {
        match key {
            "kind" => {
                request.kind = AskKind::from_key(value)
                    .ok_or_else(|| bad_choice("kind", value, "line, move or look"))?;
            }
            "prompt" => request.prompt = value.to_owned(),
            "note" => request.note = Some(value.to_owned()),
            _ => {
                let (label, rest) = value
                    .split_once('|')
                    .ok_or_else(|| bad_choice("card", value, "<label>|<line>[|<thing id>]"))?;
                let (line, thing) = match rest.split_once('|') {
                    Some((line, thing)) => (line, Some(thing.trim().to_owned())),
                    None => (rest, None),
                };
                request.cards.push(SuggestedLine {
                    label: label.trim().to_owned(),
                    line: line.trim().to_owned(),
                    thing,
                });
            }
        }
    }
    if request.prompt.trim().is_empty() {
        return Err(usage());
    }
    Ok(request)
}

/// `wait=<seconds>`: how long `players inbox` waits for a line.
fn wait_ms(text: &str) -> Result<u64, CommandError> {
    let seconds = text
        .strip_prefix("wait=")
        .ok_or(CommandError::Usage(PLAYERS_USAGE))?;
    seconds
        .parse::<u64>()
        .map(|seconds| seconds.saturating_mul(1_000))
        .map_err(|_| bad_choice("wait", seconds, "a whole number of seconds"))
}

fn number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse()
        .map_err(|_| bad_choice(what, text, "a whole number"))
}

/// Reads the words after `players`.
pub fn parse(words: &[String]) -> Result<PlayersCommand, CommandError> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["list"] => Ok(PlayersCommand::List),
        ["friend", "add", name @ ..] if !name.is_empty() => {
            Ok(PlayersCommand::FriendAdd(name.join(" ")))
        }
        ["friend", "remove", id] => number("id", id).map(PlayersCommand::FriendRemove),
        ["friend", "list"] => Ok(PlayersCommand::FriendList),
        ["pending"] => Ok(PlayersCommand::Pending),
        ["approve", request] => number("request", request).map(PlayersCommand::Approve),
        ["refuse", request] => number("request", request).map(PlayersCommand::Refuse),
        ["hero", "add", seat, species, class, name @ ..] if !name.is_empty() => {
            Ok(PlayersCommand::HeroAdd(HeroAdd {
                seat: number("seat", seat)?,
                name: name.join(" "),
                species: (*species).to_owned(),
                class: (*class).to_owned(),
            }))
        }
        ["remove", seat] => number("seat", seat).map(PlayersCommand::Remove),
        ["persona", seat, persona] => Ok(PlayersCommand::Persona(PersonaBind {
            seat: number("seat", seat)?,
            persona: (*persona != "none").then(|| (*persona).to_owned()),
        })),
        ["ask", seats, "clear"] => seats_named(seats).map(PlayersCommand::AskClear),
        ["ask", seats, ..] => {
            let to = seats_named(seats)?;
            ask(to, &words_after(words.as_slice(), 2)).map(PlayersCommand::Ask)
        }
        ["roll", seat, call] => Ok(PlayersCommand::Roll {
            seat: number("seat", seat)?,
            call: (*call).to_owned(),
        }),
        ["note", seat, _, ..] => Ok(PlayersCommand::Note(SeatNote {
            seat: number("seat", seat)?,
            text: words_after(words.as_slice(), 2).join(" "),
        })),
        ["inbox", seats] => Ok(PlayersCommand::Inbox(InboxWait {
            from: seats_named(seats)?,
            wait_ms: 0,
        })),
        ["inbox", seats, wait] => Ok(PlayersCommand::Inbox(InboxWait {
            from: seats_named(seats)?,
            wait_ms: wait_ms(wait)?,
        })),
        _ => Err(CommandError::Usage(PLAYERS_USAGE)),
    }
}

/// The words after the first `skip`, owned again for the option reader.
fn words_after(words: &[&str], skip: usize) -> Vec<String> {
    words
        .iter()
        .skip(skip)
        .map(|word| (*word).to_owned())
        .collect()
}

fn format_friends(friends: &[FriendCard]) -> Vec<String> {
    let mut lines = vec![format!("friends: {}", friends.len())];
    lines.extend(
        friends
            .iter()
            .map(|friend| format!("  friend {}  {}", friend.id, friend.name)),
    );
    lines
}

fn format_pending(report: &PlayersReport) -> Vec<String> {
    let mut lines = vec![format!("pending: {}", report.pending.len())];
    lines.extend(report.pending.iter().map(|pending| {
        format!(
            "  request {}  friend {} ({})  {}  asked_ms={}",
            pending.request,
            pending.profile,
            pending.name,
            pending.state.key(),
            pending.asked_ms
        )
    }));
    lines
}

fn format_report(report: &PlayersReport) -> String {
    let mut lines = vec![
        format!("room code: {}", report.room_code),
        format!("seats: {}", report.seats.len()),
    ];
    lines.extend(report.seats.iter().map(|seat| {
        let profile = seat
            .profile
            .map_or_else(String::new, |profile| format!(" profile={profile}"));
        format!(
            "  seat {}  {}  {}{profile}  joined_ms={} last_seen_ms={}",
            seat.seat,
            seat.name,
            seat.kind.key(),
            seat.joined_ms,
            seat.last_seen_ms
        )
    }));
    lines.extend(format_friends(&report.friends));
    lines.extend(format_pending(report));
    lines.push(format!("heroes: {}", report.heroes));
    lines.join("\n")
}

async fn post<Request: serde::Serialize, Reply: serde::de::DeserializeOwned>(
    port: u16,
    route: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    let reply: SeatReply<Reply> = post_json(port, route, request).await?;
    dev_client::reply_value(reply)
}

/// As [`post`], with the run token of the runner this command line runs for (`DF_RUN_TOKEN`), when
/// it runs for one: the host refuses a runner that is not the current day's (`not_this_run`).
async fn post_as_run<Request: serde::Serialize, Reply: serde::de::DeserializeOwned>(
    port: u16,
    route: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    let token = std::env::var(super::key::RUN_TOKEN_VARIABLE).ok();
    let headers: Vec<(&str, &str)> = token
        .as_deref()
        .map(|token| (super::key::DEV_RUN_HEADER, token))
        .into_iter()
        .collect();
    let reply: SeatReply<Reply> =
        dev_client::post_json_with(port, route, &headers, request).await?;
    dev_client::reply_value(reply)
}

async fn report(port: u16) -> Result<PlayersReport, DevError> {
    let reply: SeatReply<PlayersReport> = get_json(port, path::PLAYERS).await?;
    dev_client::reply_value(reply)
}

/// Runs one `players` command against the host on `port`.
pub async fn run(port: u16, command: PlayersCommand) -> Result<String, DevError> {
    match command {
        PlayersCommand::List => Ok(format_report(&report(port).await?)),
        PlayersCommand::FriendList => Ok(format_friends(&report(port).await?.friends).join("\n")),
        PlayersCommand::Pending => Ok(format_pending(&report(port).await?).join("\n")),
        PlayersCommand::FriendAdd(name) => {
            let friend: FriendCard = post(port, path::FRIEND_ADD, &FriendAdd { name }).await?;
            Ok(format!("friend {} added: {}", friend.id, friend.name))
        }
        PlayersCommand::FriendRemove(profile) => {
            let _: PlayersReport =
                post(port, path::FRIEND_REMOVE, &FriendRemove { profile }).await?;
            Ok(format!("friend {profile} removed"))
        }
        PlayersCommand::Approve(request) => {
            let approved: Approved = post(port, path::APPROVE, &ApprovalAnswer { request }).await?;
            Ok(format!(
                "request {request} approved: seat {} {}",
                approved.seat.seat, approved.seat.name
            ))
        }
        PlayersCommand::Refuse(request) => {
            let _: PlayersReport = post(port, path::REFUSE, &ApprovalAnswer { request }).await?;
            Ok(format!("request {request} refused"))
        }
        PlayersCommand::HeroAdd(hero) => {
            let added: HeroAdded = post(port, path::HERO_ADD, &hero).await?;
            Ok(format!(
                "hero {} added: {} ({} {})",
                added.hero.id, added.hero.name, added.hero.species, added.hero.class
            ))
        }
        PlayersCommand::Remove(seat) => {
            let _: PlayersReport = post(port, path::SEAT_REMOVE, &SeatRemove { seat }).await?;
            Ok(format!("seat {seat} removed"))
        }
        PlayersCommand::Persona(bind) => {
            let _: PlayersReport = post(port, path::PERSONA, &bind).await?;
            Ok(format!(
                "seat {} plays {}",
                bind.seat,
                bind.persona.as_deref().unwrap_or("no character")
            ))
        }
        PlayersCommand::Ask(request) => {
            let asked: Asked = post_as_run(port, path::ASK, &request).await?;
            let seats: Vec<String> = asked.seats.iter().map(u64::to_string).collect();
            Ok(format!(
                "asked ask={} seat={} cards={}",
                asked.ask,
                seats.join(","),
                request.cards.len()
            ))
        }
        PlayersCommand::Roll { seat, call } => {
            // The tap is the phone's own action, sent as the seat over the loopback listener.
            let action = SeatAction::Roll {
                said: WallClock.now_ms(),
                call,
            };
            let seat_word = seat.to_string();
            let reply: SeatActionReply = dev_client::post_json_with(
                port,
                df_playground::path::PLAY_SEAT_ACTION,
                &[(DEV_SEAT_HEADER, seat_word.as_str())],
                &action,
            )
            .await?;
            let outcome = dev_client::reply_value(reply)?;
            Ok(format!(
                "seat {seat}: {}",
                serde_json::to_string(&outcome).unwrap_or_default()
            ))
        }
        PlayersCommand::Note(note) => {
            let seat = note.seat;
            let _: Inbox = post_as_run(port, path::NOTE, &note).await?;
            Ok(format!("noted seat={seat}"))
        }
        PlayersCommand::AskClear(to) => {
            let named = match &to {
                SeatsNamed::Party => "party".to_owned(),
                SeatsNamed::Seats(seats) => seats
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            };
            let _: Inbox = post_as_run(port, path::ASK_CLEAR, &AskClear { to }).await?;
            Ok(format!("ask cleared seat={named}"))
        }
        PlayersCommand::Inbox(wait) => {
            let inbox: Inbox = post_as_run(port, path::INBOX, &wait).await?;
            Ok(match inbox {
                Inbox::Said {
                    seat,
                    ask,
                    kind,
                    thing,
                    text,
                    to,
                    at_ms,
                } => {
                    let to = to.map_or_else(String::new, |to| format!(" to={to}"));
                    let thing = thing.map_or_else(String::new, |thing| format!(" thing={thing}"));
                    match (kind, ask) {
                        (SayKind::Answer, Some(ask)) => {
                            format!("said ask={ask} at_ms={at_ms} seat={seat}{to}{thing} | {text}")
                        }
                        _ => format!("talk at_ms={at_ms} seat={seat}{to} | {text}"),
                    }
                }
                Inbox::Cast {
                    seat,
                    spell,
                    name,
                    level,
                    ritual,
                    summary,
                    at_ms,
                } => format!(
                    "cast at_ms={at_ms} seat={seat} spell={spell} level={level} ritual={ritual} \
                     | {name}: {summary}"
                ),
                Inbox::Empty => "none".to_owned(),
            })
        }
    }
}
