//! The join screens as the phone draws them: one `JoinScreen` for each state of the seat machine,
//! in the phone kit's regions (header, status, a scrolling body, a dock of buttons). The words and
//! the taps are decided here, so native tests cover them; `view::seat` only draws the result.

use crate::phone_kit::act::ActionCard;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::shell::{ButtonKind, EmptyState, LoadingState, PhoneHeader, Pill, PillTone};
use crate::seat::wire::{
    HeroCard, RoomState, SeatAction, SeatCard, SeatView, TablePlayer, TableStage,
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
    Act(SeatAction),
    /// Start again from the room.
    Retry,
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
            _ => numbered("friend-pick-")
                .map(Self::PickFriend)
                .or_else(|| numbered("hero-").map(Self::PlayHero))
                .or_else(|| {
                    word.strip_prefix("act-")
                        .and_then(SeatAction::from_key)
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
}

fn card(title: &str, text: Option<&str>, tap: SeatTap) -> JoinCard {
    JoinCard {
        card: ActionCard {
            label: title.to_owned(),
            explain: text.map(str::to_owned),
            glyph: None,
            lit: false,
        },
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
    let build = format!("{} {}", hero.species, hero.class);
    JoinCard {
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
fn roster_pill(player: &TablePlayer) -> Pill {
    let (text, tone) = match (player.present, player.ready) {
        (false, _) => (format!("{} (away)", player.name), PillTone::Neutral),
        (true, true) => (format!("{} (ready)", player.name), PillTone::Good),
        (true, false) => (player.name.clone(), PillTone::Neutral),
    };
    Pill {
        text,
        tone: if player.you { PillTone::Accent } else { tone },
        glyph: Some(PhoneGlyph::Hero),
    }
}

/// How the lobby stands, in the players' words: only who is at the table counts.
fn lobby_lead(players: &[TablePlayer]) -> String {
    let present = players.iter().filter(|player| player.present).count();
    let ready = players
        .iter()
        .filter(|player| player.present && player.ready)
        .count();
    if present > 0 && ready == present {
        "Everyone is ready. The game starts soon.".to_owned()
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
    if view.stage == TableStage::Playing {
        return None;
    }
    let mut buttons = Vec::new();
    if view.can_toggle_ready {
        buttons.push(if view.ready {
            button(
                "Not ready",
                ButtonKind::Secondary,
                SeatTap::Act(SeatAction::Unready),
            )
        } else {
            button(
                "I'm ready",
                ButtonKind::Primary,
                SeatTap::Act(SeatAction::Ready),
            )
        });
    }
    buttons.push(button(
        "Leave the table",
        ButtonKind::Danger,
        SeatTap::Act(SeatAction::Leave),
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
        roster: view.players.iter().map(roster_pill).collect(),
        notice: problem(machine),
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
        SeatState::Seated { seat, view } => return seated(machine, seat, view.as_ref()),
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
