//! The walk of a move: the sights the party passes on the way to a place are shown one after the
//! other before the place, so the party goes through the town instead of being put in it.
//!
//! Every plate (each sight, and the place) is made at the same time, the sights after the
//! movers' sheets alone, all at normal priority, so a sight costs no more than its own making and
//! the place is not made later for them. A plate goes behind the scene as soon as it is
//! ready and the one before it has been up its hold (never under [`MINIMUM_HOLD`]: nothing changes
//! faster than a person can follow). While the place is not made, the sights that landed together
//! are spread over the wait the place is expected to take ([`PLACE_TO_SIGHT`] times the sights'
//! own making), so the screen is not left on the last of them for the rest of the wait; a place
//! that comes in sooner only waits the plain hold. A sight that is not ready when it would be
//! next, and the place is ready, is passed over (`late`), so a slow sight never keeps the place
//! from arriving; the place never goes up before a sight that is ready.
//! [`Sequencer`] decides, on a clock it is given;
//! [`conduct`] runs it against the plates as they come in and a [`Screen`]; [`walk_through`] is the
//! live screen: it puts the plates behind the scene, keeps each sight as a memory of the mover,
//! posts the tempo impulse of each, reports each moment to the host and writes the transcript.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use df_types::saturating_millis;
use futures_util::future::join_all;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, sleep_until, timeout_at};

use super::brief::{BriefCharacter, LookSheets, build_stage_brief};
use super::types::{PlateBrief, RouteSight, SceneDesign, TownRole, WalkAsk};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::locate::request::CharacterRequest;
use crate::persona::{
    MadePlate, Painting, Spinner, WalkMoment, make_walk_plate, mark_walk, show_walk_plate,
    spinner_off, spinner_on,
};
use crate::tempo::request::{ImpulseRequest, TempoReply};
use crate::tempo::routes::path as tempo_path;
use crate::world::memory::request::{CaptureRequest, MemoryReply};
use crate::world::memory::routes::path as memory_path;
use crate::world::memory::{LinkKind, MemoryLink, Put};
use crate::world::request::WorldReply;
use crate::world::routes::path as world_path;
use crate::world::{WalkNoted, WalkOutcome, WalkStageNote};

/// The least time a sight on the way stays behind the scene before another picture replaces it.
pub const MINIMUM_HOLD: Duration = Duration::from_secs(5);
/// How long a sight stays up unless the caller says another time (`hold=`): a little over the
/// least, so the fade into it is over and it can still be taken in.
pub const DEFAULT_HOLD: Duration = Duration::from_secs(6);
/// How many times as long as a sight the place is expected to take to make: it carries the movers'
/// sheets, what they hold and the residents. Moves 5, 6 and 7 of the live roam run 9 measured 2.2,
/// 2.9 and 1.7. It is the least of them that is rounded, so a place is never kept waiting for it.
pub const PLACE_TO_SIGHT: u64 = 2;
/// The id of a move that is only looked at, with no plate to make.
const UNPLATED: &str = "unplated";
/// The kind of the memory each sight is kept as.
pub const MEMORY_KIND: &str = "walk";
/// The `created_by` of those memories.
const MEMORY_AUTHOR: &str = "walk";

/// A tempo impulse posted as each sight goes up: the kind the music table names and its strength
/// in permille.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impulse {
    pub kind: String,
    pub permille: u16,
}

/// What the caller of a move asks of its walk: how many sights on the way at most, the move's role
/// in its town, how long each sight stays up, and the impulse each posts (`walk=`, `town=`,
/// `hold=`, `impulse=` of `move say` and `move act`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkOptions {
    pub stages: usize,
    pub town: Option<TownRole>,
    pub hold: Option<Duration>,
    pub impulse: Option<Impulse>,
}

impl WalkOptions {
    /// What the design is asked for the move painted as `plate`: none for a move that asks no walk
    /// and no town. A move with no plate is only looked at, so it changes no town: its role is
    /// within.
    pub fn ask(&self, plate: Option<&str>) -> Option<WalkAsk> {
        (self.stages > 0 || self.town.is_some()).then(|| match plate {
            Some(plate) => WalkAsk {
                stages: self.stages,
                town: self.town.unwrap_or(TownRole::Within),
                move_id: plate.to_ascii_lowercase(),
            },
            None => WalkAsk {
                stages: self.stages,
                town: TownRole::Within,
                move_id: UNPLATED.to_owned(),
            },
        })
    }

    fn hold(&self) -> Duration {
        self.hold.unwrap_or(DEFAULT_HOLD)
    }
}

/// Why a sight was passed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// It was not ready when its turn came and the place was.
    Late,
    /// Making it, or putting it behind the scene, failed.
    Failed,
}

impl Skip {
    pub fn key(self) -> &'static str {
        match self {
            Self::Late => "late",
            Self::Failed => "failed",
        }
    }

    fn outcome(self) -> WalkOutcome {
        match self {
            Self::Late => WalkOutcome::Late,
            Self::Failed => WalkOutcome::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlateState {
    Pending,
    Ready,
    Failed,
}

/// What to do next in a walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Put sight `0..` behind the scene now.
    Stage(usize),
    /// Pass the sight over.
    Skipped { index: usize, why: Skip },
    /// Put the place behind the scene now; the walk is over.
    Arrival,
    /// Nothing to do until the clock reaches this many milliseconds, or until a plate comes in
    /// when there is none.
    Wait(Option<u64>),
    /// The place is up.
    Done,
}

/// The order and the times of a walk's pictures, on a clock in milliseconds that the caller reads.
/// Pure: it shows nothing, waits for nothing and reads no clock.
#[derive(Debug, Clone)]
pub struct Sequencer {
    stages: Vec<PlateState>,
    arrival_ready: bool,
    hold_ms: u64,
    /// When the first sight went up: how long the sights took to make.
    first_up_ms: Option<u64>,
    /// When the sight that is up has been up its plain hold.
    held_at_ms: u64,
    /// When it has been up its hold spread over the wait for the place: never before `held_at_ms`.
    spread_at_ms: u64,
    next: usize,
    arrived: bool,
}

impl Sequencer {
    /// A walk of `stages` sights, each up for `hold` at least (never under [`MINIMUM_HOLD`]).
    pub fn new(stages: usize, hold: Duration) -> Self {
        Self {
            stages: vec![PlateState::Pending; stages],
            arrival_ready: false,
            hold_ms: saturating_millis(hold.max(MINIMUM_HOLD)),
            first_up_ms: None,
            held_at_ms: 0,
            spread_at_ms: 0,
            next: 0,
            arrived: false,
        }
    }

    /// Sight `index` is made, or its making failed.
    pub fn made_stage(&mut self, index: usize, made: bool) {
        if let Some(state) = self.stages.get_mut(index) {
            *state = if made {
                PlateState::Ready
            } else {
                PlateState::Failed
            };
        }
    }

    /// The place is made.
    pub fn made_arrival(&mut self) {
        self.arrival_ready = true;
    }

    /// Whether the place is made.
    pub fn arrival_ready(&self) -> bool {
        self.arrival_ready
    }

    /// Gives up every sight still pending, which no plate will ever come for; answers their
    /// indexes. They are passed over as failed.
    pub fn give_up_stages(&mut self) -> Vec<usize> {
        let mut given_up = Vec::new();
        for (index, state) in self.stages.iter_mut().enumerate().skip(self.next) {
            if *state == PlateState::Pending {
                *state = PlateState::Failed;
                given_up.push(index);
            }
        }
        given_up
    }

    /// Where every plate stands, in words: each sight passed over or up, ready, pending or failed,
    /// and the place.
    pub fn report(&self) -> String {
        let mut parts: Vec<String> = self
            .stages
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let word = match state {
                    _ if index < self.next => "passed",
                    PlateState::Pending => "pending",
                    PlateState::Ready => "ready",
                    PlateState::Failed => "failed",
                };
                format!("stage {} {word}", index + 1)
            })
            .collect();
        let place = match (self.arrived, self.arrival_ready) {
            (true, _) => "up",
            (false, true) => "made",
            (false, false) => "pending",
        };
        parts.push(format!("place {place}"));
        parts.join(", ")
    }

    /// When the screen is free: the sight that is up has been up its hold, spread over the wait
    /// for the place while the place is not made, and plain once it is.
    fn free_at_ms(&self) -> u64 {
        if self.arrival_ready {
            self.held_at_ms
        } else {
            self.spread_at_ms
        }
    }

    /// How long the sight that goes up at `now_ms` stays up if the place is not made: its share of
    /// the time left until the place is expected, and never under the plain hold. `left` counts it
    /// and the sights after it that may still go up.
    fn spread_hold_ms(&self, first_up_ms: u64, now_ms: u64, left: usize) -> u64 {
        let expected_ms = first_up_ms.saturating_mul(PLACE_TO_SIGHT);
        let share =
            expected_ms.saturating_sub(now_ms) / u64::try_from(left).map_or(1, |n| n.max(1));
        share.max(self.hold_ms)
    }

    /// The next thing to do at `now_ms`. A sight that goes up holds the screen for the hold (spread
    /// over the wait for the place while it is not made); the sights go in order; a sight not ready
    /// when the screen is free and the place is, is passed over; the place goes up after every
    /// sight that was ready, once the screen is free.
    pub fn step(&mut self, now_ms: u64) -> Step {
        if self.arrived {
            return Step::Done;
        }
        let free = now_ms >= self.free_at_ms();
        if let Some(state) = self.stages.get(self.next).copied() {
            let index = self.next;
            return match state {
                PlateState::Failed => {
                    self.next += 1;
                    Step::Skipped {
                        index,
                        why: Skip::Failed,
                    }
                }
                PlateState::Ready if free => {
                    let left = self.stages[index..]
                        .iter()
                        .filter(|state| **state != PlateState::Failed)
                        .count();
                    let first_up_ms = *self.first_up_ms.get_or_insert(now_ms);
                    let spread_ms = self.spread_hold_ms(first_up_ms, now_ms, left);
                    self.held_at_ms = now_ms.saturating_add(self.hold_ms);
                    self.spread_at_ms = now_ms.saturating_add(spread_ms);
                    self.next += 1;
                    Step::Stage(index)
                }
                PlateState::Pending if free && self.arrival_ready => {
                    self.next += 1;
                    Step::Skipped {
                        index,
                        why: Skip::Late,
                    }
                }
                PlateState::Pending if !self.arrival_ready => Step::Wait(None),
                PlateState::Ready | PlateState::Pending => Step::Wait(Some(self.free_at_ms())),
            };
        }
        match (self.arrival_ready, free) {
            (false, _) => Step::Wait(None),
            (true, true) => {
                self.arrived = true;
                Step::Arrival
            }
            (true, false) => Step::Wait(Some(self.free_at_ms())),
        }
    }
}

/// Which picture of the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shot {
    Stage(usize),
    Arrival,
}

/// A plate made, as it comes in.
pub enum Made<P> {
    Stage(usize, Result<P, WalkFailure>),
    Arrival(Result<P, WalkFailure>),
}

/// A moment of a walk, on the clock of [`conduct`] (milliseconds since it began).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moment {
    /// The place came in made.
    Made { at_ms: u64 },
    /// A picture went behind the scene.
    Shown { shot: Shot, at_ms: u64 },
    /// Sight `stage` was replaced after being up for `up_ms`.
    Ended { stage: usize, up_ms: u64 },
    /// Sight `index` was passed over.
    Skipped {
        index: usize,
        why: Skip,
        detail: String,
    },
}

/// Why a walk could not be carried through: the key of the step that failed and its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkFailure {
    pub key: String,
    pub message: String,
}

impl fmt::Display for WalkFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.key, self.message)
    }
}

impl std::error::Error for WalkFailure {}

/// Where a walk's pictures go and what it makes of each moment.
pub trait Screen {
    type Plate;

    /// Puts `plate` behind the scene now.
    async fn show(&mut self, shot: Shot, plate: Self::Plate) -> Result<(), WalkFailure>;

    /// Takes in a moment of the walk.
    async fn note(&mut self, moment: Moment);
}

fn elapsed_ms(start: Instant) -> u64 {
    saturating_millis(start.elapsed())
}

/// Carries a walk of `stages` sights through: takes in the plates as `made` brings them, asks the
/// [`Sequencer`] what to do at each moment and does it on `screen`, until the place is up. A
/// sight whose picture cannot be put up is passed over; the place that cannot be made or put up
/// ends the walk with its failure. Every wait is on the clock of the runtime, so a paused clock
/// runs it without waiting.
pub async fn conduct<S: Screen>(
    screen: &mut S,
    mut made: UnboundedReceiver<Made<S::Plate>>,
    stages: usize,
    hold: Duration,
) -> Result<(), WalkFailure> {
    let start = Instant::now();
    let mut sequencer = Sequencer::new(stages, hold);
    let mut stage_plates: Vec<Option<S::Plate>> = (0..stages).map(|_| None).collect();
    let mut arrival_plate: Option<S::Plate> = None;
    let mut failures: Vec<String> = vec![String::new(); stages];
    let mut up: Option<(usize, u64)> = None;
    let mut closed = false;
    loop {
        match sequencer.step(elapsed_ms(start)) {
            Step::Stage(index) => {
                let Some(plate) = stage_plates[index].take() else {
                    continue;
                };
                match screen.show(Shot::Stage(index), plate).await {
                    Ok(()) => {
                        let at_ms = elapsed_ms(start);
                        if let Some((stage, shown_ms)) = up.replace((index, at_ms)) {
                            let up_ms = at_ms.saturating_sub(shown_ms);
                            screen.note(Moment::Ended { stage, up_ms }).await;
                        }
                        screen
                            .note(Moment::Shown {
                                shot: Shot::Stage(index),
                                at_ms,
                            })
                            .await;
                    }
                    Err(failure) => {
                        let moment = Moment::Skipped {
                            index,
                            why: Skip::Failed,
                            detail: failure.to_string(),
                        };
                        screen.note(moment).await;
                    }
                }
            }
            Step::Skipped { index, why } => {
                let detail = std::mem::take(&mut failures[index]);
                screen.note(Moment::Skipped { index, why, detail }).await;
            }
            Step::Arrival => {
                let Some(plate) = arrival_plate.take() else {
                    continue;
                };
                screen.show(Shot::Arrival, plate).await?;
                let at_ms = elapsed_ms(start);
                if let Some((stage, shown_ms)) = up.take() {
                    let up_ms = at_ms.saturating_sub(shown_ms);
                    screen.note(Moment::Ended { stage, up_ms }).await;
                }
                screen
                    .note(Moment::Shown {
                        shot: Shot::Arrival,
                        at_ms,
                    })
                    .await;
                return Ok(());
            }
            Step::Done => return Ok(()),
            Step::Wait(until) => {
                let received = match (closed, until) {
                    // Every plate that will ever come has come: the rest is only the hold.
                    (true, Some(ms)) => {
                        sleep_until(start + Duration::from_millis(ms)).await;
                        continue;
                    }
                    (true, None) => None,
                    (false, Some(ms)) => {
                        let deadline = start + Duration::from_millis(ms);
                        match timeout_at(deadline, made.recv()).await {
                            Ok(event) => event,
                            Err(_) => continue,
                        }
                    }
                    (false, None) => made.recv().await,
                };
                let Some(event) = received else {
                    closed = true;
                    if !sequencer.arrival_ready() {
                        return Err(WalkFailure {
                            key: "plates_stopped".to_owned(),
                            message: format!(
                                "the walk's plates stopped before the place was made: {}",
                                sequencer.report()
                            ),
                        });
                    }
                    // A sight that has not come by now never will: it is passed over.
                    for index in sequencer.give_up_stages() {
                        failures[index] = "no plate came for it".to_owned();
                    }
                    continue;
                };
                // Plates that came in together are all taken in before anything is decided, so a
                // sight that finished with the place is not passed over for being second in line.
                let mut events = vec![event];
                while let Ok(more) = made.try_recv() {
                    events.push(more);
                }
                for event in events {
                    match event {
                        Made::Stage(index, Ok(plate)) => {
                            stage_plates[index] = Some(plate);
                            sequencer.made_stage(index, true);
                        }
                        Made::Stage(index, Err(failure)) => {
                            failures[index] = failure.to_string();
                            sequencer.made_stage(index, false);
                        }
                        Made::Arrival(Ok(plate)) => {
                            arrival_plate = Some(plate);
                            sequencer.made_arrival();
                            let at_ms = elapsed_ms(start);
                            screen.note(Moment::Made { at_ms }).await;
                        }
                        Made::Arrival(Err(failure)) => return Err(failure),
                    }
                }
            }
        }
    }
}

/// What a walk came to, for the transcript and the transition's records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walked {
    /// The lines for the move's report, in order.
    pub lines: Vec<String>,
    /// When the place was made, since the walk began.
    pub made_ms: Option<u64>,
    /// When the place went up, since the walk began.
    pub arrived_ms: Option<u64>,
}

/// Everything a walk through the sights of a design needs.
pub struct Through<'a> {
    pub port: u16,
    /// The place's plate: its name in the media library, and its place id lowercased.
    pub plate: &'a str,
    pub design: &'a SceneDesign,
    /// The movers, who go through the sights.
    pub movers: &'a [BriefCharacter],
    /// Everyone in the place's picture, as the scan is asked to name them.
    pub placed: &'a [CharacterRequest],
    /// The place's own painting.
    pub arrival: Painting<'a>,
    /// Whose memories the sights are kept as, when someone said the line.
    pub owner: Option<&'a str>,
    pub options: &'a WalkOptions,
}

/// The live screen of a walk: it puts the plates behind the scene and reports what happens.
struct Live<'a> {
    port: u16,
    plate: &'a str,
    place: String,
    sights: &'a [RouteSight],
    present_stage: Vec<String>,
    present_arrival: Vec<String>,
    owner: Option<&'a str>,
    impulse: Option<&'a Impulse>,
    spinner: Option<Spinner>,
    cast_from: Option<String>,
    pictures: Vec<Option<String>>,
    shown_at_ms: Vec<Option<u64>>,
    open: BTreeSet<usize>,
    arrival_open: bool,
    first_shown_ms: Option<u64>,
    shown_count: usize,
    made_ms: Option<u64>,
    arrived_ms: Option<u64>,
    lines: Vec<String>,
}

impl Live<'_> {
    fn frame(&self, shot: Shot) -> String {
        match shot {
            Shot::Stage(index) => stage_frame(self.plate, index),
            Shot::Arrival => self.plate.to_owned(),
        }
    }

    fn label(&self, index: usize) -> String {
        format!(
            "stage {}/{} {}",
            index + 1,
            self.sights.len(),
            self.sights[index].sight
        )
    }

    async fn stage_note(&mut self, index: usize, outcome: WalkOutcome, up_ms: Option<u64>) {
        let note = WalkStageNote {
            place: self.place.clone(),
            stage: self.sights[index].id.clone(),
            number: index + 1,
            of: self.sights.len(),
            outcome,
            up_ms,
        };
        let sent: Result<WorldReply<WalkNoted>, _> =
            post_json(self.port, world_path::WORLD_WALK, &note).await;
        if let Err(error) = sent {
            self.lines
                .push(format!("walk stage note not recorded: {error}"));
        }
    }

    async fn mark(&mut self, frame: &str, moment: WalkMoment) -> Option<u64> {
        match mark_walk(self.port, frame, moment).await {
            Ok(at_ms) => Some(at_ms),
            Err(error) => {
                self.lines
                    .push(format!("walk timing of {frame} not recorded: {error}"));
                None
            }
        }
    }

    /// Posts the impulse of a sight going up; a table without it is no failure.
    async fn post_impulse(&mut self) {
        let Some(impulse) = self.impulse else {
            return;
        };
        let request = ImpulseRequest {
            kind: impulse.kind.clone(),
            intensity: Some(impulse.permille),
            note: None,
        };
        let sent: Result<TempoReply<serde_json::Value>, _> =
            post_json(self.port, tempo_path::IMPULSE, &request).await;
        match sent {
            Ok(DevReply::Done { .. }) => {}
            Ok(DevReply::Rejected { message, .. }) => self.lines.push(format!(
                "walk impulse {} not posted: {message}",
                impulse.kind
            )),
            Err(error) => self
                .lines
                .push(format!("walk impulse {} not posted: {error}", impulse.kind)),
        }
    }

    /// Keeps the sight `index`, now behind the scene, as a memory of the mover linked to the place.
    async fn remember(&mut self, index: usize, picture: &str) {
        let Some(owner) = self.owner else {
            return;
        };
        let sight = &self.sights[index].sight;
        let request = CaptureRequest {
            owner: owner.to_owned(),
            kind: MEMORY_KIND.to_owned(),
            picture: picture.to_owned(),
            title: sight.clone(),
            id: None,
            description: Some(format!("On the way to the place, passing {sight}")),
            place: Some(self.place.clone()),
            links: vec![MemoryLink {
                what: LinkKind::Place,
                id: self.place.clone(),
            }],
            tags: Vec::new(),
            created_by: Some(MEMORY_AUTHOR.to_owned()),
        };
        let sent: Result<MemoryReply<Put>, _> =
            post_json(self.port, memory_path::MEMORY_CAPTURE, &request).await;
        let line = match sent {
            Ok(DevReply::Done { value }) => format!(
                "walk memory {} kind={} seq={} place={} picture={picture}",
                value.memory.id, value.memory.kind, value.memory.seq, self.place
            ),
            Ok(DevReply::Rejected { message, .. }) => {
                format!("walk memory not captured: {message}")
            }
            Err(error) => format!("walk memory not captured: {error}"),
        };
        self.lines.push(line);
    }

    /// Closes every picture the walk started and never put up, when the walk ends in failure.
    async fn abandon(&mut self) {
        let open: Vec<usize> = std::mem::take(&mut self.open).into_iter().collect();
        for index in open {
            let frame = stage_frame(self.plate, index);
            self.mark(&frame, WalkMoment::Skipped).await;
        }
        if std::mem::take(&mut self.arrival_open) {
            let frame = self.plate.to_owned();
            self.mark(&frame, WalkMoment::Failed).await;
        }
        if let Some(spinner) = self.spinner.take()
            && let Err(error) = spinner_off(self.port, spinner).await
        {
            self.lines.push(format!("walk title not put back: {error}"));
        }
    }
}

impl Screen for Live<'_> {
    type Plate = MadePlate;

    async fn show(&mut self, shot: Shot, plate: MadePlate) -> Result<(), WalkFailure> {
        let frame = self.frame(shot);
        let picture = plate.picture().to_owned();
        let making = plate.making().unwrap_or_default();
        self.lines.push(format!(
            "walk {frame} made in {} ms: {}",
            making.as_millis(),
            plate.found()
        ));
        show_walk_plate(
            self.port,
            plate,
            &frame,
            self.cast_from.as_deref(),
            match shot {
                Shot::Stage(_) => &self.present_stage,
                Shot::Arrival => &self.present_arrival,
            },
        )
        .await
        .map_err(|(step, error)| WalkFailure {
            key: step.key().to_owned(),
            message: error.to_string(),
        })?;
        self.cast_from = Some(picture.clone());
        if let Some(spinner) = self.spinner.take()
            && let Err(error) = spinner_off(self.port, spinner).await
        {
            self.lines.push(format!("walk title not put back: {error}"));
        }
        let at_ms = self.mark(&frame, WalkMoment::Shown(making)).await;
        match shot {
            Shot::Stage(index) => {
                self.open.remove(&index);
                self.pictures[index] = Some(picture);
                self.shown_at_ms[index] = at_ms;
            }
            Shot::Arrival => self.arrival_open = false,
        }
        Ok(())
    }

    async fn note(&mut self, moment: Moment) {
        match moment {
            Moment::Made { at_ms } => self.made_ms = Some(at_ms),
            Moment::Shown {
                shot: Shot::Stage(index),
                at_ms,
            } => {
                self.first_shown_ms.get_or_insert(at_ms);
                self.shown_count += 1;
                self.post_impulse().await;
                if let Some(picture) = self.pictures[index].clone() {
                    self.remember(index, &picture).await;
                }
            }
            Moment::Shown {
                shot: Shot::Arrival,
                at_ms,
            } => self.arrived_ms = Some(at_ms),
            Moment::Ended { stage, up_ms } => {
                let frame = stage_frame(self.plate, stage);
                self.mark(&frame, WalkMoment::Ended(Duration::from_millis(up_ms)))
                    .await;
                self.stage_note(stage, WalkOutcome::Shown, Some(up_ms))
                    .await;
                let label = self.label(stage);
                let line = shown_line(&label, self.shown_at_ms[stage], up_ms);
                self.lines.push(line);
            }
            Moment::Skipped { index, why, detail } => {
                if self.open.remove(&index) {
                    let frame = stage_frame(self.plate, index);
                    let moment = match why {
                        Skip::Late => WalkMoment::Skipped,
                        Skip::Failed => WalkMoment::Failed,
                    };
                    self.mark(&frame, moment).await;
                }
                self.stage_note(index, why.outcome(), None).await;
                let label = self.label(index);
                self.lines.push(skipped_line(&label, why, &detail));
            }
        }
    }
}

/// The name of the plate of sight `index` on the way to the place painted as `plate`.
fn stage_frame(plate: &str, index: usize) -> String {
    format!("{plate}-walk{}", index + 1)
}

/// The line of a sight that was shown: when it went up (the playground's clock) and how long it was
/// up before the next picture replaced it.
pub(super) fn shown_line(label: &str, shown_at_ms: Option<u64>, up_ms: u64) -> String {
    let shown_at = shown_at_ms.map_or_else(|| "-".to_owned(), |at_ms| at_ms.to_string());
    format!("walk {label} shown_at_ms={shown_at} up_ms={up_ms}")
}

/// The line of a sight that was passed over, with why and, for a failure, what failed.
pub(super) fn skipped_line(label: &str, why: Skip, detail: &str) -> String {
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" ({detail})")
    };
    format!("walk {label} skipped reason={}{detail}", why.key())
}

/// The line that ends a walk: how many sights were shown of those asked, and the time from the
/// first sight going up to the place going up.
pub(super) fn arrived_line(shown: usize, of: usize, walk_ms: u64) -> String {
    format!("walk arrived stages={shown}/{of} walk_ms={walk_ms}")
}

// TODO(DEV-12): `walk_through` on a host with a fake image backend and scan: the swap of each
// plate, the frame timing, stage note, memory and impulse each sight posts, and the spinner.
/// Makes the sights on the way and the place together and shows them in order (see the module
/// header). On success the place is behind the scene and the walk's lines say what happened; on
/// failure the failed step's key and words, with every picture the walk started closed.
pub async fn walk_through(through: Through<'_>) -> Result<Walked, WalkFailure> {
    let Through {
        port,
        plate,
        design,
        movers,
        placed,
        arrival,
        owner,
        options,
    } = through;
    let sights = design.stages.as_slice();
    let stage_people: Vec<CharacterRequest> = placed
        .iter()
        .filter(|request| movers.iter().any(|mover| mover.id == request.name))
        .cloned()
        .collect();
    let mut live = Live {
        port,
        plate,
        place: plate.to_ascii_lowercase(),
        sights,
        present_stage: names_of(&stage_people),
        present_arrival: names_of(placed),
        owner,
        impulse: options.impulse.as_ref(),
        spinner: spinner_on(port).await.ok(),
        cast_from: None,
        pictures: vec![None; sights.len()],
        shown_at_ms: vec![None; sights.len()],
        open: (0..sights.len()).collect(),
        arrival_open: true,
        first_shown_ms: None,
        shown_count: 0,
        made_ms: None,
        arrived_ms: None,
        lines: Vec::new(),
    };
    let briefs: Vec<Result<PlateBrief, WalkFailure>> = sights
        .iter()
        .map(|sight| {
            build_stage_brief(design, sight, movers, &LookSheets::empty()).map_err(|error| {
                WalkFailure {
                    key: error.key().to_owned(),
                    message: error.to_string(),
                }
            })
        })
        .collect();
    for (index, brief) in briefs.iter().enumerate() {
        let label = live.label(index);
        live.lines.push(match brief {
            Ok(brief) => format!("walk {label} prompt: {}", brief.prompt),
            Err(failure) => format!("walk {label} brief refused: {failure}"),
        });
    }
    let (sender, receiver) = unbounded_channel();
    let stages: Vec<_> = briefs
        .iter()
        .enumerate()
        .map(|(index, brief)| make_stage(port, plate, index, brief, &stage_people, sender.clone()))
        .collect();
    let arriving = make_arrival(port, plate, &arrival, placed, sender);
    // The plates are made together; the walk ends when the place is up, which drops what is
    // still being made.
    let producing = async {
        tokio::join!(join_all(stages), arriving);
        std::future::pending::<Result<(), WalkFailure>>().await
    };
    let conducted = tokio::select! {
        conducted = conduct(&mut live, receiver, sights.len(), options.hold()) => conducted,
        never = producing => never,
    };
    if let Err(failure) = conducted {
        live.abandon().await;
        return Err(failure);
    }
    let walk_ms = match (live.first_shown_ms, live.arrived_ms) {
        (Some(first), Some(arrived)) => arrived.saturating_sub(first),
        _ => 0,
    };
    live.lines
        .push(arrived_line(live.shown_count, sights.len(), walk_ms));
    Ok(Walked {
        lines: live.lines,
        made_ms: live.made_ms,
        arrived_ms: live.arrived_ms,
    })
}

/// Makes sight `index` from its brief and sends it in; a sight whose brief was refused or whose
/// making fails is sent as failed.
async fn make_stage(
    port: u16,
    plate: &str,
    index: usize,
    brief: &Result<PlateBrief, WalkFailure>,
    people: &[CharacterRequest],
    sender: UnboundedSender<Made<MadePlate>>,
) {
    let name = stage_frame(plate, index);
    let _ = mark_walk(port, &name, WalkMoment::Started).await;
    let made = match brief {
        Ok(brief) => {
            let painting = Painting {
                prompt: &brief.prompt,
                avoid: &brief.avoid,
                sheets: &[],
            };
            make_walk_plate(port, &name, &painting, people.to_vec())
                .await
                .map_err(|(step, error)| WalkFailure {
                    key: step.key().to_owned(),
                    message: error.to_string(),
                })
        }
        Err(failure) => Err(failure.clone()),
    };
    let _ = sender.send(Made::Stage(index, made));
}

/// Makes the place's plate and sends it in.
async fn make_arrival(
    port: u16,
    plate: &str,
    painting: &Painting<'_>,
    placed: &[CharacterRequest],
    sender: UnboundedSender<Made<MadePlate>>,
) {
    let _ = mark_walk(port, plate, WalkMoment::Started).await;
    let made = make_walk_plate(port, plate, painting, placed.to_vec())
        .await
        .map_err(|(step, error)| WalkFailure {
            key: step.key().to_owned(),
            message: error.to_string(),
        });
    let _ = sender.send(Made::Arrival(made));
}

fn names_of(people: &[CharacterRequest]) -> Vec<String> {
    people.iter().map(|person| person.name.clone()).collect()
}
