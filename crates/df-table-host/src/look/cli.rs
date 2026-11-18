//! The `look` verbs of the dev command line: the frames' base poses (candidates and the owner's
//! choice), and a seat's look studio (controls, details, refine, keep, discard, start again, lock
//! in). The poses, a seat's hero and its pack are the look studio's dev routes (`routes::path`);
//! a seat's studio is the phone's own routes (`path::PLAY_LOOK*`), called as the seat
//! (`seat::key::DEV_SEAT_HEADER`). A draft is the phone's JSON
//! (`df_playground::look::wire::LookDraft`), given as the last words.

use df_playground::look::wire::{
    BodyFrame, HeroPack, LockState, LookAgainAsk, LookControls, LookDetails, LookDetailsAsk,
    LookDraft, LookLockAsk, LookPackAsk, LookRefineAsk, LookRollAsk, LookStartAsk,
    LookStartOptions, LookStudioView, LookVersion, LookVersionAsk, PackItem, VersionState,
};
use df_playground::path::{
    PLAY_LOOK, PLAY_LOOK_AGAIN, PLAY_LOOK_CONTROLS, PLAY_LOOK_DETAILS, PLAY_LOOK_DISCARD,
    PLAY_LOOK_KEEP, PLAY_LOOK_LOCK, PLAY_LOOK_REFINE, PLAY_LOOK_ROLL, PLAY_LOOK_START,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::request::{
    HeroBind, HeroLook, LookReply, PoseCandidates, PoseCandidatesAsk, PoseChoose, PoseView,
    PosesView,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, get_json_with, post_json, post_json_with};
use crate::seat::key::{DEV_SEAT_HEADER, DEV_WAIT_HEADER};

/// Every `look` verb, for the usage refusal and the help.
pub const LOOK_USAGE: &str = concat!(
    "look poses | look pose candidates <small|medium|tall> <base|sheet> [count] | ",
    "look pose choose <small|medium|tall> <base|sheet> <picture> | look hero <seat> <hero> | ",
    "look hero <seat> | ",
    "look show <seat> | look controls <seat> | look details <seat> <words...> | ",
    "look refine <seat> [kept=<version>|kept=none] [wait=true] [<draft JSON>] | ",
    "look keep <seat> <version> | look discard <seat> <version> | look again <seat> | ",
    "look lock <seat> [wait=true] [<draft JSON>] | ",
    "look start [<seat> <species> <class> <name...>] | look roll <seat> | ",
    "look pack <seat> [sheet|headshot|portrait|ready]"
);

/// Which version a refine edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptChoice {
    /// The one the studio keeps now.
    Current,
    /// This one, or none (render from the base pose).
    Set(Option<u64>),
}

/// A `look` command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookCommand {
    Poses,
    PoseCandidates(PoseCandidatesAsk),
    PoseChoose(PoseChoose),
    Hero(HeroBind),
    /// The kept look of the hero a seat plays: its details and its whole look.
    HeroLook(u64),
    Show(u64),
    Controls(u64),
    Details(u64, String),
    Refine {
        seat: u64,
        kept: KeptChoice,
        wait: bool,
        draft: LookDraft,
    },
    Keep(u64, u64),
    Discard(u64, u64),
    Again(u64),
    Lock {
        seat: u64,
        wait: bool,
        draft: LookDraft,
    },
    /// A random look for the seat's hero, printed as a draft to refine with.
    Roll(u64),
    /// Make a pack item of the seat's hero again, or every item not made.
    Pack(u64, Option<PackItem>),
    /// What a new hero may be started as.
    StartOptions,
    /// A new hero for the seat, its look studio afresh.
    Start(u64, LookStartAsk),
}

fn number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse()
        .map_err(|_| bad_choice(what, text, "a whole number"))
}

fn frame(text: &str) -> Result<BodyFrame, CommandError> {
    BodyFrame::from_key(text).ok_or_else(|| bad_choice("frame", text, "small, medium or tall"))
}

fn view(text: &str) -> Result<PoseView, CommandError> {
    PoseView::from_key(text).ok_or_else(|| bad_choice("view", text, "base or sheet"))
}

fn draft(words: &[&str]) -> Result<LookDraft, CommandError> {
    if words.is_empty() {
        return Ok(LookDraft::default());
    }
    let text = words.join(" ");
    serde_json::from_str(&text).map_err(|error| {
        bad_choice(
            "draft",
            &text,
            format!("the phone's draft as JSON, one quoted word ({error})"),
        )
    })
}

/// The `key=value` options at the front of `words`, and the words after them.
fn options<'words>(
    words: &'words [&'words str],
    keys: &[&str],
) -> (Vec<(&'words str, &'words str)>, &'words [&'words str]) {
    let taken = words
        .iter()
        .take_while(|word| {
            word.split_once('=')
                .is_some_and(|(key, _)| keys.contains(&key))
        })
        .count();
    let pairs = words[..taken]
        .iter()
        .filter_map(|word| word.split_once('='))
        .collect();
    (pairs, &words[taken..])
}

fn wait_of(text: &str) -> Result<bool, CommandError> {
    text.parse()
        .map_err(|_| bad_choice("wait", text, "true or false"))
}

fn refine(seat: &str, rest: &[&str]) -> Result<LookCommand, CommandError> {
    let (pairs, rest) = options(rest, &["kept", "wait"]);
    let mut kept = KeptChoice::Current;
    let mut wait = false;
    for (key, value) in pairs {
        match key {
            "kept" if value == "none" => kept = KeptChoice::Set(None),
            "kept" => kept = KeptChoice::Set(Some(number("kept", value)?)),
            _ => wait = wait_of(value)?,
        }
    }
    Ok(LookCommand::Refine {
        seat: number("seat", seat)?,
        kept,
        wait,
        draft: draft(rest)?,
    })
}

fn lock(seat: &str, rest: &[&str]) -> Result<LookCommand, CommandError> {
    let (pairs, rest) = options(rest, &["wait"]);
    let mut wait = false;
    for (_, value) in pairs {
        wait = wait_of(value)?;
    }
    Ok(LookCommand::Lock {
        seat: number("seat", seat)?,
        wait,
        draft: draft(rest)?,
    })
}

/// Reads the words after `look`.
pub fn parse(words: &[String]) -> Result<LookCommand, CommandError> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["poses"] => Ok(LookCommand::Poses),
        ["pose", "candidates", frame_word, view_word, count @ ..] => {
            let count = match count {
                [] => 1,
                [count] => u32::try_from(number("count", count)?)
                    .map_err(|_| bad_choice("count", count, "a whole number"))?,
                _ => return Err(CommandError::Usage(LOOK_USAGE)),
            };
            Ok(LookCommand::PoseCandidates(PoseCandidatesAsk {
                frame: frame(frame_word)?,
                view: view(view_word)?,
                count,
            }))
        }
        ["pose", "choose", frame_word, view_word, picture] => {
            Ok(LookCommand::PoseChoose(PoseChoose {
                frame: frame(frame_word)?,
                view: view(view_word)?,
                picture: (*picture).to_owned(),
            }))
        }
        ["hero", seat] => number("seat", seat).map(LookCommand::HeroLook),
        ["hero", seat, hero] => Ok(LookCommand::Hero(HeroBind {
            seat: number("seat", seat)?,
            hero: number("hero", hero)?,
        })),
        ["show", seat] => number("seat", seat).map(LookCommand::Show),
        ["controls", seat] => number("seat", seat).map(LookCommand::Controls),
        ["details", seat, words @ ..] if !words.is_empty() => {
            Ok(LookCommand::Details(number("seat", seat)?, words.join(" ")))
        }
        ["refine", seat, rest @ ..] => refine(seat, rest),
        ["keep", seat, version] => Ok(LookCommand::Keep(
            number("seat", seat)?,
            number("version", version)?,
        )),
        ["discard", seat, version] => Ok(LookCommand::Discard(
            number("seat", seat)?,
            number("version", version)?,
        )),
        ["again", seat] => number("seat", seat).map(LookCommand::Again),
        ["lock", seat, rest @ ..] => lock(seat, rest),
        ["start"] => Ok(LookCommand::StartOptions),
        ["roll", seat] => number("seat", seat).map(LookCommand::Roll),
        ["pack", seat] => Ok(LookCommand::Pack(number("seat", seat)?, None)),
        ["pack", seat, item] => {
            Ok(LookCommand::Pack(
                number("seat", seat)?,
                Some(PackItem::from_key(item).ok_or_else(|| {
                    bad_choice("item", item, "sheet, headshot, portrait or ready")
                })?),
            ))
        }
        ["start", seat, species, class, name @ ..] if !name.is_empty() => Ok(LookCommand::Start(
            number("seat", seat)?,
            LookStartAsk {
                name: name.join(" "),
                species: (*species).to_owned(),
                class: (*class).to_owned(),
            },
        )),
        _ => Err(CommandError::Usage(LOOK_USAGE)),
    }
}

fn format_version(version: &LookVersion, kept: Option<u64>) -> String {
    let mark = if kept == Some(version.id) {
        " (kept)"
    } else {
        ""
    };
    let cause = match version.state {
        VersionState::Failed { cause } => format!(" cause={}", cause.key()),
        _ => String::new(),
    };
    let mut line = format!(
        "  v{}{mark}  {}{cause}  parent={}  picture={}  took_ms={}",
        version.id,
        version.state.key(),
        version
            .parent
            .map_or_else(|| "none".to_owned(), |parent| format!("v{parent}")),
        version.picture.as_deref().unwrap_or("-"),
        version
            .took_ms
            .map_or_else(|| "-".to_owned(), |took| took.to_string()),
    );
    for change in &version.changes {
        line.push_str(&format!("\n      {change}"));
    }
    line
}

fn format_studio(view: &LookStudioView) -> String {
    let mut lines = vec![format!(
        "kept: {}  versions: {}",
        view.kept
            .map_or_else(|| "none".to_owned(), |kept| format!("v{kept}")),
        view.versions.len()
    )];
    lines.extend(
        view.versions
            .iter()
            .map(|version| format_version(version, view.kept)),
    );
    if let Some(lock) = &view.lock {
        let state = match &lock.state {
            LockState::Portrait => "making the portrait".to_owned(),
            LockState::Sheet { portrait } => format!(
                "making the sheet from {}",
                portrait.as_deref().unwrap_or("no portrait")
            ),
            LockState::Locked { portrait, sheet } => format!(
                "locked: portrait={} sheet={}",
                portrait.as_deref().unwrap_or("none"),
                sheet.as_deref().unwrap_or("failed")
            ),
        };
        lines.push(format!("lock: hero {}  {state}", lock.hero));
    }
    lines.join("\n")
}

fn format_controls(controls: &LookControls) -> String {
    let mut lines = vec![format!("frame: {}", controls.frame.key())];
    lines.extend(controls.controls.iter().map(|control| {
        let kind = serde_json::to_string(&control.kind).unwrap_or_default();
        let default = serde_json::to_string(&control.default).unwrap_or_default();
        format!("  {}  default={default}  {kind}", control.id)
    }));
    lines.join("\n")
}

fn format_poses(poses: &PosesView) -> String {
    let mut lines = vec![format!("frames: {}", poses.frames.len())];
    lines.extend(poses.frames.iter().map(|(frame, chosen)| {
        format!(
            "  {}  base={} (from {})  sheet={} (from {})",
            frame.key(),
            chosen.base.as_deref().unwrap_or("none"),
            chosen.base_from.as_deref().unwrap_or("-"),
            chosen.sheet.as_deref().unwrap_or("none"),
            chosen.sheet_from.as_deref().unwrap_or("-"),
        )
    }));
    lines.join("\n")
}

async fn post<Request: Serialize, Reply: DeserializeOwned>(
    port: u16,
    route: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    let reply: LookReply<Reply> = post_json(port, route, request).await?;
    dev_client::reply_value(reply)
}

async fn get<Reply: DeserializeOwned>(port: u16, route: &str) -> Result<Reply, DevError> {
    let reply: LookReply<Reply> = get_json(port, route).await?;
    dev_client::reply_value(reply)
}

/// Posts `request` to `route` as the seat `seat`, waiting for the picture it starts when `wait`.
async fn post_as<Request: Serialize, Reply: DeserializeOwned>(
    port: u16,
    (seat, wait): (u64, bool),
    route: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    let seat = seat.to_string();
    let mut headers = vec![(DEV_SEAT_HEADER, seat.as_str())];
    if wait {
        headers.push((DEV_WAIT_HEADER, "true"));
    }
    let reply: LookReply<Reply> = post_json_with(port, route, &headers, request).await?;
    dev_client::reply_value(reply)
}

/// Reads `route` as the seat `seat`.
async fn get_as<Reply: DeserializeOwned>(
    port: u16,
    seat: u64,
    route: &str,
) -> Result<Reply, DevError> {
    let seat = seat.to_string();
    let reply: LookReply<Reply> =
        get_json_with(port, route, &[(DEV_SEAT_HEADER, seat.as_str())]).await?;
    dev_client::reply_value(reply)
}

async fn studio(port: u16, seat: u64) -> Result<LookStudioView, DevError> {
    get_as(port, seat, PLAY_LOOK).await
}

/// Runs one `look` command against the host on `port`.
pub async fn run(port: u16, command: LookCommand) -> Result<String, DevError> {
    match command {
        LookCommand::Poses => Ok(format_poses(&get(port, path::POSES).await?)),
        LookCommand::PoseCandidates(ask) => {
            let made: PoseCandidates = post(port, path::POSE_CANDIDATES, &ask).await?;
            Ok(format!(
                "asked for {} (jobs {:?}); see media generate status, then look pose choose",
                made.names.join(", "),
                made.jobs
            ))
        }
        LookCommand::PoseChoose(ask) => {
            Ok(format_poses(&post(port, path::POSE_CHOOSE, &ask).await?))
        }
        LookCommand::HeroLook(seat) => {
            let look: HeroLook = get(port, &format!("{}?seat={seat}", path::HERO)).await?;
            Ok(format!(
                "hero={}\ndetails={}\nlook={}",
                look.hero,
                look.details.as_deref().unwrap_or("-"),
                look.look.as_deref().unwrap_or("-")
            ))
        }
        LookCommand::Hero(bind) => {
            let bound: HeroBind = post(port, path::HERO, &bind).await?;
            Ok(format!("seat {} plays hero {}", bound.seat, bound.hero))
        }
        LookCommand::Show(seat) => Ok(format_studio(&studio(port, seat).await?)),
        LookCommand::Controls(seat) => {
            let controls: LookControls = get_as(port, seat, PLAY_LOOK_CONTROLS).await?;
            Ok(format_controls(&controls))
        }
        LookCommand::Details(seat, words) => {
            let read: LookDetails = post_as(
                port,
                (seat, false),
                PLAY_LOOK_DETAILS,
                &LookDetailsAsk { words },
            )
            .await?;
            Ok(serde_json::to_string_pretty(&read.details).unwrap_or_default())
        }
        LookCommand::Refine {
            seat,
            kept,
            wait,
            draft,
        } => {
            let kept = match kept {
                KeptChoice::Current => studio(port, seat).await?.kept,
                KeptChoice::Set(kept) => kept,
            };
            let ask = LookRefineAsk { draft, kept };
            let version: LookVersion = post_as(port, (seat, wait), PLAY_LOOK_REFINE, &ask).await?;
            Ok(format_version(&version, kept))
        }
        LookCommand::Keep(seat, version) => {
            let ask = LookVersionAsk { version };
            Ok(format_studio(
                &post_as(port, (seat, false), PLAY_LOOK_KEEP, &ask).await?,
            ))
        }
        LookCommand::Discard(seat, version) => {
            let ask = LookVersionAsk { version };
            Ok(format_studio(
                &post_as(port, (seat, false), PLAY_LOOK_DISCARD, &ask).await?,
            ))
        }
        LookCommand::Again(seat) => Ok(format_studio(
            &post_as(port, (seat, false), PLAY_LOOK_AGAIN, &LookAgainAsk {}).await?,
        )),
        LookCommand::Pack(seat, item) => {
            let pack: HeroPack =
                post_as(port, (seat, false), path::PACK, &LookPackAsk { item }).await?;
            let lines: Vec<String> = PackItem::ALL
                .into_iter()
                .map(|item| {
                    let status = pack.get(item);
                    format!(
                        "  {}  {}  {}",
                        item.key(),
                        status.key(),
                        status.picture().unwrap_or("-")
                    )
                })
                .collect();
            Ok(format!(
                "pack of seat {seat}'s hero:
{}",
                lines.join(
                    "
"
                )
            ))
        }
        LookCommand::Roll(seat) => {
            let draft: LookDraft =
                post_as(port, (seat, false), PLAY_LOOK_ROLL, &LookRollAsk {}).await?;
            Ok(serde_json::to_string(&draft).unwrap_or_default())
        }
        LookCommand::StartOptions => {
            let options: LookStartOptions = get(port, PLAY_LOOK_START).await?;
            let ids = |options: &[df_playground::look::wire::StartCard]| {
                options
                    .iter()
                    .map(|option| option.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Ok(format!(
                "species: {}
classes: {}",
                ids(&options.species),
                ids(&options.classes)
            ))
        }
        LookCommand::Start(seat, ask) => {
            let view: LookStudioView = post_as(port, (seat, false), PLAY_LOOK_START, &ask).await?;
            Ok(format!(
                "seat {seat} starts a new hero
{}",
                format_studio(&view)
            ))
        }
        LookCommand::Lock { seat, wait, draft } => {
            let ask = LookLockAsk { draft };
            Ok(format_studio(
                &post_as(port, (seat, wait), PLAY_LOOK_LOCK, &ask).await?,
            ))
        }
    }
}
