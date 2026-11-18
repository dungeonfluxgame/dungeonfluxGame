//! The join screens as the phone draws them: one `JoinScreen` for each state of the seat machine,
//! in the phone kit's regions (header, status, a scrolling body, a dock of buttons). The words and
//! the taps are decided here, so native tests cover them; `view::seat` only draws the result.

use crate::look::studio::{LookViewer, ViewItem};
use crate::look::wire::PackStatus;
use crate::phone_kit::act::ActionCard;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::shell::{ButtonKind, EmptyState, LoadingState, PhoneHeader, Pill, PillTone};
use crate::seat::wire::{
    DayEnd, HeroCard, LobbyStep, RoomState, SeatCard, SeatView, TableHero, TablePlayer, TableStage,
};
use crate::seat::{RefusedReason, SeatMachine, SeatState};

/// A tap on a join screen, carried by its element as `data-action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatTap {
    Continue,
    Guest,
    Friend,
    SignIn,
    Back,
    /// Join as a guest with the name in the name field.
    JoinGuest,
    PickFriend(u64),
    PlayHero(u64),
    MakeHero,
    Act(LobbyStep),
    /// Start again from the room.
    Retry,
    /// Say the line of suggested card `index` of what the table asks now.
    Say(usize),
    /// Roll the dice the dungeon master called for (`SeatView::pending_roll`).
    Roll,
    /// Take the room code typed in the code field.
    EnterCode,
}

const TAP_PREFIX: &str = "seat:";

impl SeatTap {
    /// The `data-action` of the tap.
    pub fn action(self) -> String {
        let word = match self {
            Self::Continue => "continue".to_owned(),
            Self::Guest => "guest".to_owned(),
            Self::Friend => "friend".to_owned(),
            Self::SignIn => "sign-in".to_owned(),
            Self::Back => "back".to_owned(),
            Self::JoinGuest => "join-guest".to_owned(),
            Self::PickFriend(id) => format!("friend-pick-{id}"),
            Self::PlayHero(id) => format!("hero-{id}"),
            Self::MakeHero => "new-hero".to_owned(),
            Self::Act(action) => format!("act-{}", action.key()),
            Self::Retry => "retry".to_owned(),
            Self::Say(index) => format!("say-{index}"),
            Self::Roll => "roll".to_owned(),
            Self::EnterCode => "enter-code".to_owned(),
        };
        format!("{TAP_PREFIX}{word}")
    }

    /// The tap a `data-action` names, or `None` for another control's.
    pub fn from_action(action: &str) -> Option<Self> {
        let word = action.strip_prefix(TAP_PREFIX)?;
        let numbered = |prefix: &str| word.strip_prefix(prefix)?.parse::<u64>().ok();
        match word {
            "continue" => Some(Self::Continue),
            "guest" => Some(Self::Guest),
            "friend" => Some(Self::Friend),
            "sign-in" => Some(Self::SignIn),
            "back" => Some(Self::Back),
            "join-guest" => Some(Self::JoinGuest),
            "new-hero" => Some(Self::MakeHero),
            "retry" => Some(Self::Retry),
            "roll" => Some(Self::Roll),
            "enter-code" => Some(Self::EnterCode),
            _ => numbered("friend-pick-")
                .map(Self::PickFriend)
                .or_else(|| numbered("hero-").map(Self::PlayHero))
                .or_else(|| {
                    numbered("say-")
                        .and_then(|index| usize::try_from(index).ok())
                        .map(Self::Say)
                })
                .or_else(|| {
                    word.strip_prefix("act-")
                        .and_then(LobbyStep::from_key)
                        .map(Self::Act)
                }),
        }
    }
}

/// The room at the top of the first screens: the adventure's picture and name, who has joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomBanner {
    pub title: String,
    pub picture: Option<String>,
    pub players: Vec<String>,
}

/// A tappable card (a way in, a friend, a hero): the kit's action card, a pill beside it and its
/// tap.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinCard {
    pub card: ActionCard,
    /// A face shown in the card's mark: a hero's headshot.
    pub picture: Option<String>,
    pub mark: Option<Pill>,
    pub tap: SeatTap,
}

/// A button of the dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinButton {
    pub label: String,
    pub kind: ButtonKind,
    pub tap: SeatTap,
}

/// One join screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JoinScreen {
    pub header: Option<PhoneHeader>,
    pub status: Option<Pill>,
    pub room: Option<RoomBanner>,
    pub heading: Option<String>,
    pub lead: Option<String>,
    pub cards: Vec<JoinCard>,
    /// Who is at the table, as pills that wrap.
    pub roster: Vec<Pill>,
    /// Whether the guest's name field is shown.
    pub name_field: bool,
    pub empty: Option<EmptyState>,
    pub loading: Option<LoadingState>,
    /// The host's sentence for a refusal of this screen.
    pub notice: Option<Pill>,
    pub buttons: Vec<JoinButton>,
    /// The player's own hero, large, on the lobby screen.
    pub showcase: Option<Showcase>,
    /// The other players at the table, as small cards.
    pub table: Vec<TableCard>,
    /// A picture of the player's hero open full screen (`lobby_viewer`).
    pub viewer: Option<LookViewer>,
}

/// The player's own hero on the lobby screen: the ready picture (the portrait while it is made),
/// the name, species and class, and the seat's ready state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Showcase {
    pub picture: Option<String>,
    pub name: String,
    pub build: String,
    pub ready: bool,
    /// The ready picture is being made: the portrait shows with the loader.
    pub making: bool,
    /// What a tap on the picture asks for: it opens full screen.
    pub action: Option<String>,
}

/// Another player at the table: their hero's picture, their name and their ready state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCard {
    pub name: String,
    pub picture: Option<String>,
    pub ready: bool,
    pub away: bool,
}

/// A tap of the lobby's full-screen viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyTap {
    View(String),
    Close,
}

const LOBBY_PREFIX: &str = "lobby:";

impl LobbyTap {
    pub fn action(&self) -> String {
        match self {
            Self::View(picture) => format!("{LOBBY_PREFIX}view:{picture}"),
            Self::Close => format!("{LOBBY_PREFIX}view-close"),
        }
    }

    /// The tap an action names, or `None` for another control's.
    pub fn from_action(action: &str) -> Option<Self> {
        match action.strip_prefix(LOBBY_PREFIX)? {
            "view-close" => Some(Self::Close),
            word => word
                .strip_prefix("view:")
                .filter(|picture| !picture.is_empty())
                .map(|picture| Self::View(picture.to_owned())),
        }
    }
}

/// A key with its first letter capital: a species or class as a player reads it.
fn titled(word: &str) -> String {
    let mut letters = word.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// A hero's species and class as every hero card says them: "Elf · Ranger".
fn build_line(species: &str, class: &str) -> String {
    format!("{} \u{b7} {}", titled(species), titled(class))
}

/// The pictures of a hero the lobby's viewer steps through: ready, portrait, then the sheet.
fn hero_pictures(hero: &TableHero) -> Vec<(String, &'static str)> {
    [
        (hero.pack.ready.picture(), "Ready"),
        (hero.pack.portrait.picture(), "Portrait"),
        (hero.portrait.as_deref(), "Kept look"),
        (hero.pack.headshot.picture(), "Headshot"),
        (hero.pack.sheet.picture(), "Reference sheet"),
    ]
    .into_iter()
    .filter_map(|(picture, label)| Some((picture?.to_owned(), label)))
    .collect()
}

fn own_hero(view: &SeatView) -> Option<&TableHero> {
    view.players
        .iter()
        .find(|player| player.you)
        .and_then(|player| player.hero.as_ref())
}

/// The play screen's header for a seated phone: the player's own hero, never the scenario's.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroHeader {
    pub header: PhoneHeader,
    /// What a tap on it asks for: the hero's portrait open full screen (the lobby's viewer).
    pub action: Option<String>,
}

/// The header of the play screen for `view`: the hero's face in the medallion (the headshot made
/// at lock-in, or the approved portrait), their name, and species and class; a tap opens the
/// approved portrait full screen. `None` before the player chose a hero.
pub fn hero_header(view: &SeatView) -> Option<HeroHeader> {
    let hero = own_hero(view)?;
    Some(HeroHeader {
        header: PhoneHeader {
            name: hero.name.clone(),
            subtitle: Some(build_line(&hero.species, &hero.class)),
            // The headshot made at lock-in fits the medallion; the approved portrait stands in
            // until it is ready.
            portrait: hero
                .pack
                .headshot
                .picture()
                .map(str::to_owned)
                .or_else(|| hero.portrait.clone()),
            hp: None,
        },
        action: hero
            .portrait
            .clone()
            .map(|picture| LobbyTap::View(picture).action()),
    })
}

fn showcase(view: &SeatView) -> Option<Showcase> {
    let hero = own_hero(view)?;
    let picture = hero
        .pack
        .ready
        .picture()
        .map(str::to_owned)
        .or_else(|| hero.portrait.clone());
    Some(Showcase {
        action: picture
            .clone()
            .map(|picture| LobbyTap::View(picture).action()),
        picture,
        name: hero.name.clone(),
        build: build_line(&hero.species, &hero.class),
        ready: view.ready,
        making: hero.pack.ready == PackStatus::Pending,
    })
}

fn table_cards(view: &SeatView) -> Vec<TableCard> {
    view.players
        .iter()
        .filter(|player| !player.you)
        .map(|player| TableCard {
            name: player
                .hero
                .as_ref()
                .map_or_else(|| player.name.clone(), |hero| hero.name.clone()),
            picture: player.hero.as_ref().and_then(|hero| {
                hero.pack
                    .headshot
                    .picture()
                    .map(str::to_owned)
                    .or_else(|| hero.portrait.clone())
            }),
            ready: player.ready,
            away: !player.present,
        })
        .collect()
}

/// The lobby's viewer on `picture`, one of the pictures of the player's own hero.
pub fn lobby_viewer(view: &SeatView, picture: &str) -> Option<LookViewer> {
    let pictures = hero_pictures(own_hero(view)?);
    let index = pictures.iter().position(|(known, _)| known == picture)?;
    let step = |at: Option<usize>| {
        at.and_then(|at| pictures.get(at))
            .map(|(picture, _)| LobbyTap::View(picture.clone()).action())
    };
    let (_, label) = pictures.get(index)?;
    Some(LookViewer {
        previous: step(index.checked_sub(1)),
        next: step(index.checked_add(1)),
        item: ViewItem {
            picture: picture.to_owned(),
            label: (*label).to_owned(),
            buttons: Vec::new(),
        },
        index,
        count: pictures.len(),
        close: LobbyTap::Close.action(),
    })
}

fn card(title: &str, text: Option<&str>, tap: SeatTap) -> JoinCard {
    JoinCard {
        card: ActionCard {
            label: title.to_owned(),
            explain: text.map(str::to_owned),
            glyph: None,
            lit: false,
        },
        picture: None,
        mark: None,
        tap,
    }
}

fn button(label: &str, kind: ButtonKind, tap: SeatTap) -> JoinButton {
    JoinButton {
        label: label.to_owned(),
        kind,
        tap,
    }
}

fn back() -> JoinButton {
    button("Back", ButtonKind::Secondary, SeatTap::Back)
}

fn loading(text: &str) -> Option<LoadingState> {
    Some(LoadingState {
        text: text.to_owned(),
    })
}

fn room_title(machine: &SeatMachine) -> String {
    machine
        .room()
        .and_then(|room| room.adventure.clone())
        .unwrap_or_else(|| "Tonight's table".to_owned())
}

/// The adventure at the top of a screen before the player is seated.
fn room_header(machine: &SeatMachine) -> PhoneHeader {
    PhoneHeader {
        name: room_title(machine),
        subtitle: Some("Joining the table".to_owned()),
        portrait: machine.room().and_then(|room| room.picture.clone()),
        hp: None,
    }
}

/// The player at the top of a screen once seated.
fn seat_header(machine: &SeatMachine, seat: &SeatCard) -> PhoneHeader {
    PhoneHeader {
        name: seat.name.clone(),
        subtitle: Some(room_title(machine)),
        portrait: None,
        hp: None,
    }
}

fn banner(machine: &SeatMachine) -> Option<RoomBanner> {
    let room = machine.room()?;
    Some(RoomBanner {
        title: room_title(machine),
        picture: room.picture.clone(),
        players: room.players.clone(),
    })
}

fn problem(machine: &SeatMachine) -> Option<Pill> {
    machine.problem().map(|text| Pill {
        text: text.to_owned(),
        tone: PillTone::Danger,
        glyph: Some(PhoneGlyph::Warning),
    })
}

/// "Room code": asked before a new way in (Guest, Friend, Sign in), unless the link carried it.
/// `problem` says why the last code was not taken. The page draws the code field into it.
pub fn code_screen(machine: &SeatMachine, problem: Option<&str>) -> JoinScreen {
    JoinScreen {
        header: Some(room_header(machine)),
        heading: Some("Room code".to_owned()),
        lead: Some("Type the code shown on the table's screen.".to_owned()),
        notice: problem.map(|text| Pill {
            text: text.to_owned(),
            tone: PillTone::Danger,
            glyph: Some(PhoneGlyph::Warning),
        }),
        buttons: vec![button("Next", ButtonKind::Primary, SeatTap::EnterCode)],
        ..JoinScreen::default()
    }
}

fn who_are_you(machine: &SeatMachine) -> JoinScreen {
    let mut cards = Vec::new();
    if let Some(seat) = machine.returning() {
        let mut returning = card(
            &format!("Continue as {}", seat.name),
            Some("Back to your seat at this table."),
            SeatTap::Continue,
        );
        returning.card.lit = true;
        cards.push(returning);
    }
    cards.push(card(
        "Play as a guest",
        Some("No account. Your heroes stay in this phone's browser; clearing it loses them."),
        SeatTap::Guest,
    ));
    cards.push(card(
        "A friend of this table",
        Some("Pick your name from the host's friends; the host lets you in."),
        SeatTap::Friend,
    ));
    cards.push(card(
        "Sign in",
        Some("Play with your own account."),
        SeatTap::SignIn,
    ));
    let gathering = machine
        .room()
        .is_none_or(|room| room.state == RoomState::Gathering);
    JoinScreen {
        room: banner(machine),
        status: (!gathering).then(|| Pill {
            text: "The game has started".to_owned(),
            tone: PillTone::Warn,
            glyph: None,
        }),
        heading: Some("Who are you?".to_owned()),
        cards,
        notice: problem(machine),
        ..JoinScreen::default()
    }
}

fn friends_screen(machine: &SeatMachine) -> JoinScreen {
    let cards: Vec<JoinCard> = machine
        .friends()
        .iter()
        .map(|friend| card(&friend.name, None, SeatTap::PickFriend(friend.id)))
        .collect();
    let empty = cards.is_empty().then(|| EmptyState {
        title: "No friends here yet".to_owned(),
        text: "Ask the host to add your name to this table's friends.".to_owned(),
        glyph: Some(PhoneGlyph::Hero),
    });
    JoinScreen {
        header: Some(room_header(machine)),
        heading: Some("Which friend are you?".to_owned()),
        cards,
        empty,
        notice: problem(machine),
        buttons: vec![back()],
        ..JoinScreen::default()
    }
}

fn hero_card(hero: &HeroCard) -> JoinCard {
    let build = build_line(&hero.species, &hero.class);
    JoinCard {
        // The headshot made at lock-in, else the approved portrait.
        picture: hero
            .pack
            .headshot
            .picture()
            .map(str::to_owned)
            .or_else(|| hero.portrait.clone()),
        mark: Some(Pill {
            text: "Play this hero".to_owned(),
            tone: PillTone::Accent,
            glyph: None,
        }),
        ..card(&hero.name, Some(&build), SeatTap::PlayHero(hero.id))
    }
}

fn choose_hero(machine: &SeatMachine, seat: &SeatCard, heroes: Option<&[HeroCard]>) -> JoinScreen {
    let mut screen = JoinScreen {
        header: Some(seat_header(machine, seat)),
        heading: Some("Your hero".to_owned()),
        buttons: vec![button(
            "Make a new hero",
            ButtonKind::Primary,
            SeatTap::MakeHero,
        )],
        ..JoinScreen::default()
    };
    match heroes {
        None => screen.loading = loading("Finding your heroes"),
        Some([]) => {
            screen.empty = Some(EmptyState {
                title: "No heroes yet".to_owned(),
                text: "Make your first hero for this table.".to_owned(),
                glyph: Some(PhoneGlyph::Hero),
            });
        }
        Some(heroes) => screen.cards = heroes.iter().map(hero_card).collect(),
    }
    screen
}

/// A player of the roster: lit for this phone's own seat, marked ready, or marked away when they
/// hold a seat the table does not have now.
/// How many players are at the table and how many of them are ready. A player who holds a seat
/// but is away is not at the table, so they neither count nor hold up a start.
fn readiness(players: &[TablePlayer]) -> (usize, usize) {
    let present = players.iter().filter(|player| player.present).count();
    let ready = players
        .iter()
        .filter(|player| player.present && player.ready)
        .count();
    (present, ready)
}

/// How the lobby stands, in the players' words: only who is at the table counts.
fn lobby_lead(players: &[TablePlayer]) -> String {
    let (present, ready) = readiness(players);
    if present > 0 && ready == present {
        "Everyone is ready.".to_owned()
    } else {
        format!("Waiting for everyone to be ready: {ready} of {present} ready.")
    }
}

fn seated(machine: &SeatMachine, seat: &SeatCard, view: Option<&SeatView>) -> Option<JoinScreen> {
    let Some(view) = view else {
        return Some(JoinScreen {
            header: Some(seat_header(machine, seat)),
            loading: loading("Taking your seat"),
            ..JoinScreen::default()
        });
    };
    // A seat that plays a character is in play: its story and its asks are the play screen's.
    if view.stage == TableStage::Playing || view.story.is_some() {
        return None;
    }
    let mut buttons = Vec::new();
    let (present, ready) = readiness(&view.players);
    if view.can_start && present > 0 && ready == present {
        buttons.push(button(
            "Start the game",
            ButtonKind::Primary,
            SeatTap::Act(LobbyStep::Start),
        ));
    }
    if view.can_toggle_ready {
        buttons.push(if view.ready {
            button(
                "Not ready",
                ButtonKind::Secondary,
                SeatTap::Act(LobbyStep::Unready),
            )
        } else {
            button(
                "I'm ready",
                ButtonKind::Primary,
                SeatTap::Act(LobbyStep::Ready),
            )
        });
    }
    buttons.push(button(
        "Leave the table",
        ButtonKind::Danger,
        SeatTap::Act(LobbyStep::Leave),
    ));
    Some(JoinScreen {
        header: Some(seat_header(machine, seat)),
        status: Some(if view.ready {
            Pill {
                text: "Ready".to_owned(),
                tone: PillTone::Good,
                glyph: None,
            }
        } else {
            Pill {
                text: "Not ready".to_owned(),
                tone: PillTone::Warn,
                glyph: None,
            }
        }),
        heading: Some("At the table".to_owned()),
        lead: Some(lobby_lead(&view.players)),
        showcase: showcase(view),
        table: table_cards(view),
        // How the last day ended stays in view at the table until the next day starts; a start
        // that failed says why.
        notice: problem(machine).or_else(|| {
            let failed = view.start_problem.as_ref().map(|said| Pill {
                text: said.clone(),
                tone: PillTone::Warn,
                glyph: None,
            });
            failed.or_else(|| {
                view.day_end.map(|end| {
                    let (said, next) = end.player_words();
                    Pill {
                        text: format!("{said} {next}"),
                        tone: if end == DayEnd::Finished {
                            PillTone::Neutral
                        } else {
                            PillTone::Warn
                        },
                        glyph: None,
                    }
                })
            })
        }),
        buttons,
        ..JoinScreen::default()
    })
}

fn ended(heading: &str, lead: &str) -> JoinScreen {
    JoinScreen {
        heading: Some(heading.to_owned()),
        lead: Some(lead.to_owned()),
        buttons: vec![button("Try again", ButtonKind::Primary, SeatTap::Retry)],
        ..JoinScreen::default()
    }
}

/// The join screen for the machine's state, or `None` once the seat's table has moved on from
/// the lobby (the play screen shows then).
pub fn screen_of(machine: &SeatMachine) -> Option<JoinScreen> {
    Some(match machine.state() {
        SeatState::Room => JoinScreen {
            room: banner(machine),
            loading: loading("Finding the table"),
            ..JoinScreen::default()
        },
        SeatState::WhoAreYou => who_are_you(machine),
        SeatState::Guest => JoinScreen {
            header: Some(room_header(machine)),
            heading: Some("Your name".to_owned()),
            lead: Some("The name the table calls you by.".to_owned()),
            name_field: true,
            notice: problem(machine),
            buttons: vec![
                button("Join the table", ButtonKind::Primary, SeatTap::JoinGuest),
                back(),
            ],
            ..JoinScreen::default()
        },
        SeatState::Friend => friends_screen(machine),
        SeatState::SigningIn { answer } => JoinScreen {
            header: Some(room_header(machine)),
            heading: Some("Sign in".to_owned()),
            loading: answer.is_none().then(|| LoadingState {
                text: "Asking the table".to_owned(),
            }),
            notice: answer.as_ref().map(|text| Pill {
                text: text.clone(),
                tone: PillTone::Warn,
                glyph: Some(PhoneGlyph::Warning),
            }),
            buttons: vec![back()],
            ..JoinScreen::default()
        },
        SeatState::AwaitingApproval { request } => JoinScreen {
            header: Some(room_header(machine)),
            heading: Some("Waiting for the host".to_owned()),
            lead: Some(format!(
                "The host lets you in from their screen (request {request})."
            )),
            loading: loading("Waiting for the host"),
            ..JoinScreen::default()
        },
        SeatState::ChooseHero { seat, heroes } => choose_hero(machine, seat, heroes.as_deref()),
        SeatState::Creation { seat } => JoinScreen {
            header: Some(seat_header(machine, seat)),
            heading: Some("Creation comes next".to_owned()),
            lead: Some(
                "Making a hero on the phone is the next part of the table to be built.".to_owned(),
            ),
            buttons: vec![button(
                "Back to my heroes",
                ButtonKind::Secondary,
                SeatTap::Back,
            )],
            ..JoinScreen::default()
        },
        SeatState::Seated { seat, view } => return seated(machine, seat, view.as_deref()),
        SeatState::Refused(reason) => match reason {
            RefusedReason::ByHost => ended(
                "Not this time",
                "The host did not let this phone in. Ask them, then try again.",
            ),
            RefusedReason::TableFull => ended("The table is full", "Every seat is taken."),
            RefusedReason::GameStarted => ended(
                "The game has started",
                "A new player cannot take a seat now. Ask the host.",
            ),
        },
        SeatState::RoomEnded => ended("This table is closed", "Ask the host to open it again."),
    })
}
