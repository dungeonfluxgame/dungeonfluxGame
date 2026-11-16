//! The phone's identity and seat machine (`planning/phone-and-creation.md`, "State machines",
//! phone machine 2): `room` > `who_are_you` > `guest` | `friend` > `awaiting_approval` |
//! `signing_in` > `choose_hero` > `seated`, ended by `refused` or `room_ended`.
//!
//! It owns presentation only. Every move that matters follows a host answer (`wire`): the host
//! decides the seat and the kind from the phone's key, the approval or the sign-in, and the phone
//! never picks either. A player's own taps only move between the screens of one step (Guest,
//! Friend, Sign in, Back, which hero). An event a state does not take is ignored and reported
//! `false`, never a panic. It is plain data; the page draws it (`view::seat`) and asks the host
//! what `poll` names.

#[cfg(debug_assertions)]
pub mod samples;
pub mod screen;
pub mod wire;

use self::wire::{
    FriendCard, HelloReply, HeroCard, JoinStatus, RoomReply, RoomState, SeatCard, SeatErrorKind,
    SeatIntentReply, SeatView,
};

/// Why a phone cannot take a seat now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedReason {
    /// The host said no to the friend's request.
    ByHost,
    TableFull,
    GameStarted,
}

/// Where the phone stands on its way to a seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatState {
    /// Loading the room and asking who this phone is.
    Room,
    /// "Who are you?": Continue as (when the key holds a seat), Guest, Friend, Sign in.
    WhoAreYou,
    /// A guest types their name.
    Guest,
    /// A friend picks their profile.
    Friend,
    /// Signing in; the dev host answers that it comes later (`answer`).
    SigningIn {
        answer: Option<String>,
    },
    /// The host has not answered the friend's request numbered `request` yet.
    AwaitingApproval {
        request: u64,
    },
    /// Seated; the player picks a hero or makes a new one (`heroes` is `None` until loaded).
    ChooseHero {
        seat: SeatCard,
        heroes: Option<Vec<HeroCard>>,
    },
    /// "Make a new hero": creation comes next (P2, P3).
    Creation {
        seat: SeatCard,
    },
    /// At the table, in the lobby; `view` is the seat's latest view.
    Seated {
        seat: SeatCard,
        view: Option<SeatView>,
    },
    Refused(RefusedReason),
    /// The table is not running.
    RoomEnded,
}

impl SeatState {
    /// The state's name, as the plan and the page's log say it.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::WhoAreYou => "who_are_you",
            Self::Guest => "guest",
            Self::Friend => "friend",
            Self::SigningIn { .. } => "signing_in",
            Self::AwaitingApproval { .. } => "awaiting_approval",
            Self::ChooseHero { .. } => "choose_hero",
            Self::Creation { .. } => "creation",
            Self::Seated { .. } => "seated",
            Self::Refused(_) => "refused",
            Self::RoomEnded => "room_ended",
        }
    }
}

/// What happened: a host answer, or one of the player's taps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatEvent {
    RoomLoaded(RoomReply),
    HelloAnswered(HelloReply),
    ChoseGuest,
    ChoseFriend,
    ChoseSignIn,
    Back,
    /// The host's answer to a join or to a status poll.
    JoinAnswered(JoinStatus),
    /// The host refused a join, a hello or a seat call: the kind it words, and its sentence.
    Refused {
        kind: Option<SeatErrorKind>,
        message: String,
    },
    HeroesLoaded(Vec<HeroCard>),
    /// The host recorded the hero the player chose for their seat.
    HeroChosen(SeatCard),
    MakeHero,
    ViewLoaded(SeatView),
    IntentAnswered(SeatIntentReply),
}

/// What the page asks the host for in the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatPoll {
    /// The room, then hello.
    Room,
    JoinStatus,
    Heroes,
    SeatView,
}

/// The machine: its state and what the host has said so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatMachine {
    state: SeatState,
    room: Option<RoomReply>,
    /// The seat the key holds, offered as "Continue as <name>".
    returning: Option<SeatCard>,
    friends: Vec<FriendCard>,
    /// The host's sentence for the last refusal of the screen shown (a name it refused, say).
    problem: Option<String>,
}

impl Default for SeatMachine {
    /// A phone that has just loaded: in the room.
    fn default() -> Self {
        Self {
            state: SeatState::Room,
            room: None,
            returning: None,
            friends: Vec::new(),
            problem: None,
        }
    }
}

impl SeatMachine {
    pub fn state(&self) -> &SeatState {
        &self.state
    }

    pub fn room(&self) -> Option<&RoomReply> {
        self.room.as_ref()
    }

    pub fn returning(&self) -> Option<&SeatCard> {
        self.returning.as_ref()
    }

    pub fn friends(&self) -> &[FriendCard] {
        &self.friends
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// What the page asks the host for now, if anything.
    pub fn poll(&self) -> Option<SeatPoll> {
        match self.state() {
            SeatState::Room => Some(SeatPoll::Room),
            SeatState::AwaitingApproval { .. } => Some(SeatPoll::JoinStatus),
            SeatState::ChooseHero { heroes: None, .. } => Some(SeatPoll::Heroes),
            SeatState::Seated { .. } => Some(SeatPoll::SeatView),
            _ => None,
        }
    }

    fn go(&mut self, state: SeatState) -> bool {
        self.problem = None;
        self.state = state;
        true
    }

    /// Applies `event`; `false` when the current state does not take it.
    pub fn apply(&mut self, event: SeatEvent) -> bool {
        let state = self.state.clone();
        match (state, event) {
            (SeatState::Room, SeatEvent::RoomLoaded(room)) => {
                let closed = room.state == RoomState::Closed;
                self.room = Some(room);
                if closed {
                    return self.go(SeatState::RoomEnded);
                }
                true
            }
            (_, SeatEvent::RoomLoaded(room)) => {
                self.room = Some(room);
                true
            }
            (SeatState::Room, SeatEvent::HelloAnswered(hello)) => self.hello(hello),
            (SeatState::WhoAreYou, SeatEvent::ChoseGuest) => self.go(SeatState::Guest),
            (SeatState::WhoAreYou, SeatEvent::ChoseFriend) => self.go(SeatState::Friend),
            (SeatState::WhoAreYou, SeatEvent::ChoseSignIn) => {
                self.go(SeatState::SigningIn { answer: None })
            }
            (
                SeatState::Guest | SeatState::Friend | SeatState::SigningIn { .. },
                SeatEvent::Back,
            ) => self.go(SeatState::WhoAreYou),
            (SeatState::Creation { seat }, SeatEvent::Back) => {
                self.go(SeatState::ChooseHero { seat, heroes: None })
            }
            (state, SeatEvent::JoinAnswered(status)) => self.joined(&state, status),
            (state, SeatEvent::Refused { kind, message }) => self.refused(&state, kind, message),
            (SeatState::ChooseHero { seat, .. }, SeatEvent::HeroesLoaded(heroes)) => {
                self.go(SeatState::ChooseHero {
                    seat,
                    heroes: Some(heroes),
                })
            }
            (SeatState::ChooseHero { .. }, SeatEvent::HeroChosen(seat)) => {
                seat.hero.is_some() && self.seated(seat)
            }
            (SeatState::ChooseHero { seat, .. }, SeatEvent::MakeHero) => {
                self.go(SeatState::Creation { seat })
            }
            (SeatState::Seated { seat, .. }, SeatEvent::ViewLoaded(view)) => {
                self.state = SeatState::Seated {
                    seat,
                    view: Some(view),
                };
                true
            }
            (SeatState::Seated { .. }, SeatEvent::IntentAnswered(reply)) => match reply {
                SeatIntentReply::Left => {
                    self.returning = None;
                    self.go(SeatState::WhoAreYou)
                }
                SeatIntentReply::Declined { reason } => {
                    self.problem = Some(reason);
                    true
                }
                SeatIntentReply::Applied => {
                    self.problem = None;
                    true
                }
            },
            _ => false,
        }
    }

    fn hello(&mut self, hello: HelloReply) -> bool {
        match hello {
            HelloReply::Seated { seat } => self.seated(seat),
            HelloReply::Returning { seat, friends } => {
                self.returning = Some(seat);
                self.friends = friends;
                self.go(SeatState::WhoAreYou)
            }
            HelloReply::Unknown { friends } => {
                self.returning = None;
                self.friends = friends;
                self.go(SeatState::WhoAreYou)
            }
        }
    }

    /// A seat the host gave: a player who has chosen a hero for it goes to the table, waiting for
    /// its first view; one who has not chooses (or makes) a hero first.
    fn seated(&mut self, seat: SeatCard) -> bool {
        self.returning = Some(seat.clone());
        if seat.hero.is_none() {
            return self.go(SeatState::ChooseHero { seat, heroes: None });
        }
        self.go(SeatState::Seated { seat, view: None })
    }

    fn joined(&mut self, from: &SeatState, status: JoinStatus) -> bool {
        let asking = matches!(
            from,
            SeatState::WhoAreYou | SeatState::Guest | SeatState::Friend
        );
        let waiting = matches!(from, SeatState::AwaitingApproval { .. });
        match status {
            JoinStatus::Seated { seat } if asking || waiting => self.seated(seat),
            JoinStatus::AwaitingApproval { request } if *from == SeatState::Friend || waiting => {
                self.go(SeatState::AwaitingApproval { request })
            }
            JoinStatus::Refused if waiting => self.go(SeatState::Refused(RefusedReason::ByHost)),
            JoinStatus::NotAsked if waiting => self.go(SeatState::WhoAreYou),
            _ => false,
        }
    }

    fn refused(&mut self, from: &SeatState, kind: Option<SeatErrorKind>, message: String) -> bool {
        match kind {
            Some(SeatErrorKind::TableClosed) => self.go(SeatState::RoomEnded),
            Some(SeatErrorKind::TableFull) => self.go(SeatState::Refused(RefusedReason::TableFull)),
            Some(SeatErrorKind::GameStarted) => {
                self.go(SeatState::Refused(RefusedReason::GameStarted))
            }
            Some(SeatErrorKind::AccountsNotAvailable)
                if matches!(from, SeatState::SigningIn { .. }) =>
            {
                self.state = SeatState::SigningIn {
                    answer: Some(message),
                };
                true
            }
            Some(SeatErrorKind::UnknownKey)
                if matches!(
                    from,
                    SeatState::Seated { .. } | SeatState::ChooseHero { .. }
                ) =>
            {
                self.returning = None;
                self.go(SeatState::WhoAreYou)
            }
            _ => {
                self.problem = Some(message);
                true
            }
        }
    }
}
