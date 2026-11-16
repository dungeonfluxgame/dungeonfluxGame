//! The `players` verbs of the dev command line: the seat book, the friends book, the requests a
//! friend's phone made, and sample heroes. They call the seat dev routes (`routes::path`).

use df_playground::seat::wire::FriendCard;

use super::request::{
    ApprovalAnswer, Approved, FriendAdd, FriendRemove, HeroAdd, HeroAdded, PlayersReport,
    SeatRemove, SeatReply,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

/// Every `players` verb, for the usage refusal and the help.
pub const PLAYERS_USAGE: &str = concat!(
    "players [list] | players friend add <name> | players friend remove <id> | ",
    "players friend list | players pending | players approve <request> | ",
    "players refuse <request> | players hero add <seat> <species> <class> <name> | ",
    "players remove <seat>"
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
        _ => Err(CommandError::Usage(PLAYERS_USAGE)),
    }
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
    let mut lines = vec![format!("seats: {}", report.seats.len())];
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
    }
}
