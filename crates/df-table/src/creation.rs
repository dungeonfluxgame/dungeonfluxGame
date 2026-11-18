//! Creation: the table machine between the lobby and play (`planning/phone-and-creation.md` P2).
//!
//! The lobby hands its seats to creation (`CreationTable::enter` reads the seats of the lobby's
//! hand-off); each seat builds its hero, and creation hands the table to play once every seat is
//! ready (`CreationTable::start`, a hand-off to `explore` carrying the seats). Each seat is in one
//! of three states: `drafting` (the build is not yet a legal one), `reviewing` (the rules derived a
//! full sheet from it, shown for review) and `ready` (the player submitted it). A ready seat may
//! reopen until the start. A seat's hero (its name, species and class) comes from the look studio
//! that started it (`HeroStart`): the build begins from that species and class, one surface, never
//! a second chooser.
//!
//! The machine holds only its own facts: each seat's state and draft and the last derivation. It
//! never derives a number: every pick is sent to the rules through the `BuildRules` port, which
//! answers with the sheet (in the shape the check judge reads, carried here unread) and the
//! design-only marks, or with the build's problems. It never draws a die: "Surprise me" takes the
//! caller's choice of index. Each change answers a `CreationTransition` for the host's log. It
//! declines (never panics on) a step its state does not allow, with a typed `CreationError`.
//!
//! The registry's `Machine` form waits on a payload-bearing intent: `MachineInput::Intent` carries
//! an action id only, and a pick needs a slot and an option. Until then the host drives this
//! machine directly, as the dev CLI does.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::element::NameError;
use crate::identity::SeatId;
use crate::limits::MAXIMUM_DRAFT_PICKS;
use crate::listed::listed_enum;
use crate::machine::{CarryItem, Entry, HandOff, HandOffError, MachineId};

/// The creation machine's id.
pub const MACHINE_ID: &str = "creation";
/// The machine creation hands the table to, named by id only (machines never import one another).
const NEXT_MACHINE: &str = "explore";

listed_enum! {
    /// A seat's state in creation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SeatCreationState {
        Drafting,
        Reviewing,
        Ready,
    }
}

impl SeatCreationState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Drafting => "drafting",
            Self::Reviewing => "reviewing",
            Self::Ready => "ready",
        }
    }
}

listed_enum! {
    /// What moved a seat: the step the seat (or the host for it) took.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CreationCause {
        Begin,
        Choose,
        Surprise,
        /// "Random unless chosen": the picks kept, every other slot filled.
        Fill,
        Submit,
        Reopen,
        Leave,
    }
}

impl CreationCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Begin => "begin",
            Self::Choose => "choose",
            Self::Surprise => "surprise",
            Self::Fill => "fill",
            Self::Submit => "submit",
            Self::Reopen => "reopen",
            Self::Leave => "leave",
        }
    }
}

/// The hero a seat builds, as the look studio started it: its id in the hero library, its name,
/// the species and class chosen there, its portrait once locked in, and its public story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroStart {
    pub hero: u64,
    pub name: String,
    pub species: String,
    pub class: String,
    pub portrait: Option<String>,
    pub public_story: Vec<String>,
}

/// A seat's picks: each choice slot and the option id picked for it, and the slots the player took
/// back. Ids only, never a number the rules derive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildDraft {
    picks: BTreeMap<String, String>,
    /// The slots the player cleared: "random unless chosen" never fills them again; they stay
    /// empty until the player picks for them or asks for a whole new build ("Surprise me").
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    cleared: BTreeSet<String>,
}

impl BuildDraft {
    /// A draft of `picks`, refused past `MAXIMUM_DRAFT_PICKS`.
    pub fn new(picks: BTreeMap<String, String>) -> Result<Self, CreationError> {
        if picks.len() > MAXIMUM_DRAFT_PICKS {
            return Err(CreationError::TooManyPicks);
        }
        Ok(Self {
            picks,
            cleared: BTreeSet::new(),
        })
    }

    pub fn picks(&self) -> &BTreeMap<String, String> {
        &self.picks
    }

    /// The slots the player took back, which a fill leaves empty.
    pub fn cleared(&self) -> &BTreeSet<String> {
        &self.cleared
    }

    /// The draft that starts a hero: the species and class the look studio chose.
    fn starting(hero: &HeroStart) -> Self {
        let mut picks = BTreeMap::new();
        picks.insert(SPECIES_SLOT.to_owned(), hero.species.clone());
        picks.insert(CLASS_SLOT.to_owned(), hero.class.clone());
        Self {
            picks,
            cleared: BTreeSet::new(),
        }
    }
}

/// The slots the look studio's start fills.
const SPECIES_SLOT: &str = "species";
const CLASS_SLOT: &str = "class";

/// The sheet the rules derived, in the shape the check judge reads. Creation carries it unread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SheetDocument(serde_json::Value);

impl SheetDocument {
    pub fn new(sheet: serde_json::Value) -> Self {
        Self(sheet)
    }

    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}

/// One part of the build designed but not played yet, as the rules named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignMark {
    pub subject: String,
    /// The rules' closed reason key (`not_in_play_yet`, `source_not_admitted`).
    pub reason: String,
    pub note: String,
}

/// One problem of a draft, as the rules named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIssue {
    /// The slot the problem is about; `None` for the name.
    pub slot: Option<String>,
    pub sentence: String,
}

/// What the rules said of a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "derivation", rename_all = "snake_case")]
pub enum Derivation {
    /// A legal build: its sheet and what is not in play yet.
    Complete {
        sheet: SheetDocument,
        marks: Vec<DesignMark>,
    },
    /// Not a legal build yet: every problem.
    Incomplete { issues: Vec<BuildIssue> },
}

impl Derivation {
    fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    fn counts(&self) -> (usize, usize) {
        match self {
            Self::Complete { marks, .. } => (0, marks.len()),
            Self::Incomplete { issues } => (issues.len(), 0),
        }
    }
}

/// The rules owner's side of creation: the offers' derivation and "Surprise me". Implemented by
/// the host over `df-rules`, and by a fake in the tests.
pub trait BuildRules: Send + Sync {
    /// The sheet of `hero` built from `draft`, or the draft's problems.
    fn derive(&self, hero: &HeroStart, draft: &BuildDraft) -> Derivation;

    /// The picks of a full legal build from the draft's species and class; `pick` chooses an
    /// index below the count it is given.
    fn surprise(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String>;

    /// The picks of a full legal build keeping the draft's picks the rules still allow, every
    /// other slot filled ("random unless chosen").
    fn fill(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String>;
}

/// One seat in creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatCreation {
    /// The hero; `None` until the look studio starts one for the seat.
    pub hero: Option<HeroStart>,
    pub state: SeatCreationState,
    pub draft: BuildDraft,
    /// The last derivation of the draft; `None` before the seat has a hero.
    pub derivation: Option<Derivation>,
}

/// A seat's change, for the host's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationTransition {
    pub seat: SeatId,
    pub cause: CreationCause,
    /// The state before; `None` for a seat that was not in creation.
    pub from: Option<SeatCreationState>,
    /// The state after; `None` for a seat that left.
    pub to: Option<SeatCreationState>,
    /// The number of problems and design-only marks of the derivation after the step.
    pub issues: usize,
    pub marks: usize,
}

/// One finished hero: the seat, the hero (its look: species, class and portrait; and its public
/// story), the sheet and the design-only marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeroOutput {
    pub seat: SeatId,
    pub hero: HeroStart,
    pub sheet: SheetDocument,
    pub marks: Vec<DesignMark>,
}

/// What starting play hands on: the hand-off to play and one sheet per hero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationOutcome {
    pub hand_off: HandOff,
    pub heroes: Vec<HeroOutput>,
}

/// A seat as the public (the TV) sees it: its hero's name once chosen, and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicSeat {
    pub seat: SeatId,
    pub name: Option<String>,
    pub state: SeatCreationState,
}

/// Why creation declined a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationError {
    /// The seat is not in creation.
    UnknownSeat {
        seat: SeatId,
    },
    /// The seat has no hero yet: the look studio starts one.
    NoHero {
        seat: SeatId,
    },
    /// The seat is ready: it reopens before it changes its build.
    SeatReady {
        seat: SeatId,
    },
    /// Only a seat whose build the rules derived in full may submit.
    BuildIncomplete {
        seat: SeatId,
        issues: usize,
    },
    /// Only a ready seat reopens.
    NotReady {
        seat: SeatId,
    },
    /// Play has started: creation is over.
    Started,
    /// Play starts only when every seat is ready; the first that is not.
    SeatsNotReady {
        seat: SeatId,
    },
    /// The lobby handed on no seat.
    NoSeats,
    TooManyPicks,
    HandOff(HandOffError),
    Name(NameError),
}

impl fmt::Display for CreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not in creation"),
            Self::NoHero { seat } => write!(
                formatter,
                "seat {seat} has no hero yet: start one in the look studio"
            ),
            Self::SeatReady { seat } => {
                write!(
                    formatter,
                    "seat {seat} is ready: reopen it to change the build"
                )
            }
            Self::BuildIncomplete { seat, issues } => write!(
                formatter,
                "seat {seat}'s build is not complete ({issues} problems)"
            ),
            Self::NotReady { seat } => write!(formatter, "seat {seat} is not ready"),
            Self::Started => formatter.write_str("play has started: creation is over"),
            Self::SeatsNotReady { seat } => {
                write!(
                    formatter,
                    "every seat must be ready first; seat {seat} is not"
                )
            }
            Self::NoSeats => formatter.write_str("the lobby handed on no seat"),
            Self::TooManyPicks => write!(
                formatter,
                "a build draft holds at most {MAXIMUM_DRAFT_PICKS} picks"
            ),
            Self::HandOff(error) => error.fmt(formatter),
            Self::Name(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CreationError {}

/// Creation for every seat the lobby handed on, and every finished build by its hero. It is its
/// own saved record: the host keeps it whole between steps.
///
/// A build belongs to its hero: a seat holds the build of the hero it plays now (`take_up`), and a
/// finished build is kept by hero id when the seat leaves, changes hero or the next start opens
/// (`leave`, `reopen_for_next`), so the hero comes back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreationTable {
    seats: BTreeMap<SeatId, SeatCreation>,
    started: bool,
    /// The finished builds of the heroes no seat plays now, by hero id.
    #[serde(default)]
    kept: BTreeMap<u64, SeatCreation>,
}

impl CreationTable {
    /// Creation for the seats of the lobby's hand-off, each drafting with no hero yet.
    pub fn enter(entry: &Entry) -> Result<Self, CreationError> {
        let seats: BTreeMap<SeatId, SeatCreation> = entry
            .seats()
            .map(|seat| {
                (
                    seat,
                    SeatCreation {
                        hero: None,
                        state: SeatCreationState::Drafting,
                        draft: BuildDraft::default(),
                        derivation: None,
                    },
                )
            })
            .collect();
        if seats.is_empty() {
            return Err(CreationError::NoSeats);
        }
        Ok(Self {
            seats,
            started: false,
            kept: BTreeMap::new(),
        })
    }

    /// Keeps `creation` by its hero when its build is finished (the rules derived it in full); an
    /// unfinished draft is let go.
    fn keep(&mut self, creation: SeatCreation) {
        let finished = matches!(creation.derivation, Some(Derivation::Complete { .. }));
        if let (true, Some(hero)) = (finished, creation.hero.as_ref()) {
            self.kept.insert(hero.hero, creation);
        }
    }

    /// Every build: the seats', then the kept ones.
    pub fn builds(&self) -> impl Iterator<Item = &SeatCreation> {
        self.seats.values().chain(self.kept.values())
    }

    /// The build of hero `hero`: the seat's that plays it, or the one kept for it.
    pub fn build_of(&self, hero: u64) -> Option<&SeatCreation> {
        self.seats
            .values()
            .find(|creation| {
                creation
                    .hero
                    .as_ref()
                    .is_some_and(|known| known.hero == hero)
            })
            .or_else(|| self.kept.get(&hero))
    }

    /// The lobby gathers again after play: creation opens for the next start. Every seat's finished
    /// build is kept by its hero and the seats begin anew. Whether it was started.
    pub fn reopen_for_next(&mut self) -> bool {
        if !self.started {
            return false;
        }
        let seats = std::mem::take(&mut self.seats);
        for creation in seats.into_values() {
            self.keep(creation);
        }
        self.started = false;
        true
    }

    /// The seat plays `hero` now. Nothing changes when its build is already this hero's; else the
    /// build it held is kept by its hero, and the seat takes up `hero`'s kept build as it was left
    /// (a finished build is not made again), or begins a new one. Answers the transition and
    /// whether the build is new (to be filled), or none when nothing changed.
    pub fn take_up(
        &mut self,
        seat: SeatId,
        hero: HeroStart,
        rules: &dyn BuildRules,
    ) -> Result<Option<(CreationTransition, bool)>, CreationError> {
        self.admit(seat)?;
        let current = self
            .seats
            .get(&seat)
            .and_then(|creation| creation.hero.as_ref())
            .map(|known| known.hero);
        if current == Some(hero.hero) {
            return Ok(None);
        }
        if let Some(old) = self.seats.remove(&seat) {
            self.keep(old);
        }
        let Some(mut kept) = self.kept.remove(&hero.hero) else {
            self.admit(seat)?;
            return Ok(Some((self.begin(seat, hero, rules)?, true)));
        };
        let (issues, marks) = kept.derivation.as_ref().map_or((0, 0), Derivation::counts);
        let state = kept.state;
        kept.hero = Some(hero);
        self.seats.insert(seat, kept);
        Ok(Some((
            CreationTransition {
                seat,
                cause: CreationCause::Begin,
                from: Some(SeatCreationState::Drafting),
                to: Some(state),
                issues,
                marks,
            },
            false,
        )))
    }

    pub fn seat(&self, seat: SeatId) -> Option<&SeatCreation> {
        self.seats.get(&seat)
    }

    /// A seat that joined the table after creation began, drafting with no hero yet. A seat already
    /// in creation is left as it is.
    pub fn admit(&mut self, seat: SeatId) -> Result<(), CreationError> {
        if self.started {
            return Err(CreationError::Started);
        }
        self.seats.entry(seat).or_insert_with(|| SeatCreation {
            hero: None,
            state: SeatCreationState::Drafting,
            draft: BuildDraft::default(),
            derivation: None,
        });
        Ok(())
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Every seat as the TV sees it: the name once the hero is started, and the state.
    pub fn public_seats(&self) -> Vec<PublicSeat> {
        self.seats
            .iter()
            .map(|(seat, creation)| PublicSeat {
                seat: *seat,
                name: creation.hero.as_ref().map(|hero| hero.name.clone()),
                state: creation.state,
            })
            .collect()
    }

    fn open_seat(&mut self, seat: SeatId) -> Result<&mut SeatCreation, CreationError> {
        if self.started {
            return Err(CreationError::Started);
        }
        self.seats
            .get_mut(&seat)
            .ok_or(CreationError::UnknownSeat { seat })
    }

    /// Starts (or restarts) `seat`'s build from `hero`: the draft holds the species and class the
    /// look studio chose, and the rules derive it.
    pub fn begin(
        &mut self,
        seat: SeatId,
        hero: HeroStart,
        rules: &dyn BuildRules,
    ) -> Result<CreationTransition, CreationError> {
        let creation = self.open_seat(seat)?;
        if creation.state == SeatCreationState::Ready {
            return Err(CreationError::SeatReady { seat });
        }
        let draft = BuildDraft::starting(&hero);
        creation.hero = Some(hero);
        Ok(redraft(seat, creation, draft, CreationCause::Begin, rules))
    }

    /// Picks `option` for `slot` in `seat`'s draft and derives it again.
    pub fn choose(
        &mut self,
        seat: SeatId,
        slot: &str,
        option: &str,
        rules: &dyn BuildRules,
    ) -> Result<CreationTransition, CreationError> {
        let creation = editable(self.open_seat(seat)?, seat)?;
        let mut picks = creation.draft.picks().clone();
        picks.insert(slot.to_owned(), option.to_owned());
        let mut draft = BuildDraft::new(picks)?;
        draft.cleared = creation.draft.cleared.clone();
        draft.cleared.remove(slot);
        Ok(redraft(seat, creation, draft, CreationCause::Choose, rules))
    }

    /// Takes `slot` out of `seat`'s draft, kept empty from a fill, and derives it again.
    pub fn clear(
        &mut self,
        seat: SeatId,
        slot: &str,
        rules: &dyn BuildRules,
    ) -> Result<CreationTransition, CreationError> {
        let creation = editable(self.open_seat(seat)?, seat)?;
        let mut picks = creation.draft.picks().clone();
        picks.remove(slot);
        let mut draft = BuildDraft::new(picks)?;
        draft.cleared = creation.draft.cleared.clone();
        draft.cleared.insert(slot.to_owned());
        Ok(redraft(seat, creation, draft, CreationCause::Choose, rules))
    }

    /// "Surprise me": the rules propose a full build from the seat's species and class.
    pub fn surprise(
        &mut self,
        seat: SeatId,
        rules: &dyn BuildRules,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Result<CreationTransition, CreationError> {
        let creation = editable(self.open_seat(seat)?, seat)?;
        let draft = BuildDraft::new(rules.surprise(&creation.draft, pick))?;
        Ok(redraft(
            seat,
            creation,
            draft,
            CreationCause::Surprise,
            rules,
        ))
    }

    /// "Random unless chosen": the rules keep the seat's picks they still allow and fill the slots
    /// the player never set; a slot the player cleared stays empty.
    pub fn fill(
        &mut self,
        seat: SeatId,
        rules: &dyn BuildRules,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Result<CreationTransition, CreationError> {
        let creation = editable(self.open_seat(seat)?, seat)?;
        let cleared = creation.draft.cleared.clone();
        let mut picks = rules.fill(&creation.draft, pick);
        picks.retain(|slot, _| !cleared.contains(slot));
        let mut draft = BuildDraft::new(picks)?;
        draft.cleared = cleared;
        Ok(redraft(seat, creation, draft, CreationCause::Fill, rules))
    }

    /// The seat's reviewed build becomes its hero: `reviewing` to `ready`.
    pub fn submit(&mut self, seat: SeatId) -> Result<CreationTransition, CreationError> {
        let creation = editable(self.open_seat(seat)?, seat)?;
        let (issues, marks) = creation
            .derivation
            .as_ref()
            .map_or((0, 0), Derivation::counts);
        if creation.state != SeatCreationState::Reviewing {
            return Err(CreationError::BuildIncomplete { seat, issues });
        }
        creation.state = SeatCreationState::Ready;
        Ok(CreationTransition {
            seat,
            cause: CreationCause::Submit,
            from: Some(SeatCreationState::Reviewing),
            to: Some(SeatCreationState::Ready),
            issues,
            marks,
        })
    }

    /// A ready seat goes back to `reviewing`, until play starts.
    pub fn reopen(&mut self, seat: SeatId) -> Result<CreationTransition, CreationError> {
        let creation = self.open_seat(seat)?;
        if creation.state != SeatCreationState::Ready {
            return Err(CreationError::NotReady { seat });
        }
        creation.state = SeatCreationState::Reviewing;
        let (issues, marks) = creation
            .derivation
            .as_ref()
            .map_or((0, 0), Derivation::counts);
        Ok(CreationTransition {
            seat,
            cause: CreationCause::Reopen,
            from: Some(SeatCreationState::Ready),
            to: Some(SeatCreationState::Reviewing),
            issues,
            marks,
        })
    }

    /// The seat left the table: it is out of creation, and its hero's finished build is kept for
    /// the hero. A seat not in creation leaves nothing.
    pub fn leave(&mut self, seat: SeatId) -> CreationTransition {
        let left = self.seats.remove(&seat);
        let from = left.as_ref().map(|creation| creation.state);
        if let Some(left) = left {
            self.keep(left);
        }
        CreationTransition {
            seat,
            cause: CreationCause::Leave,
            from,
            to: None,
            issues: 0,
            marks: 0,
        }
    }

    /// Hands the table to play when every seat is ready: one sheet per hero, and the hand-off to
    /// `explore` carrying the seats. Declines with the first seat that is not ready.
    pub fn start(&mut self) -> Result<CreationOutcome, CreationError> {
        if self.started {
            return Err(CreationError::Started);
        }
        let mut heroes = Vec::with_capacity(self.seats.len());
        for (seat, creation) in &self.seats {
            let not_ready = CreationError::SeatsNotReady { seat: *seat };
            if creation.state != SeatCreationState::Ready {
                return Err(not_ready);
            }
            let (Some(hero), Some(Derivation::Complete { sheet, marks })) =
                (&creation.hero, &creation.derivation)
            else {
                return Err(not_ready);
            };
            heroes.push(HeroOutput {
                seat: *seat,
                hero: hero.clone(),
                sheet: sheet.clone(),
                marks: marks.clone(),
            });
        }
        if heroes.is_empty() {
            return Err(CreationError::NoSeats);
        }
        let next = MachineId::new(NEXT_MACHINE).map_err(CreationError::Name)?;
        let carry = self.seats.keys().copied().map(CarryItem::Seat).collect();
        let hand_off = HandOff::new(next, carry).map_err(CreationError::HandOff)?;
        self.started = true;
        Ok(CreationOutcome { hand_off, heroes })
    }
}

/// The seat, if its build may change now: it has a hero and is not ready.
fn editable(creation: &mut SeatCreation, seat: SeatId) -> Result<&mut SeatCreation, CreationError> {
    if creation.hero.is_none() {
        return Err(CreationError::NoHero { seat });
    }
    if creation.state == SeatCreationState::Ready {
        return Err(CreationError::SeatReady { seat });
    }
    Ok(creation)
}

/// Puts `draft` in the seat, derives it, and moves the seat to `reviewing` when the build is
/// complete and to `drafting` when it is not.
fn redraft(
    seat: SeatId,
    creation: &mut SeatCreation,
    draft: BuildDraft,
    cause: CreationCause,
    rules: &dyn BuildRules,
) -> CreationTransition {
    let from = creation.state;
    let derivation = creation
        .hero
        .as_ref()
        .map(|hero| rules.derive(hero, &draft));
    let complete = derivation.as_ref().is_some_and(Derivation::is_complete);
    let (issues, marks) = derivation.as_ref().map_or((0, 0), Derivation::counts);
    creation.state = if complete {
        SeatCreationState::Reviewing
    } else {
        SeatCreationState::Drafting
    };
    creation.draft = draft;
    creation.derivation = derivation;
    CreationTransition {
        seat,
        cause,
        from: Some(from),
        to: Some(creation.state),
        issues,
        marks,
    }
}
