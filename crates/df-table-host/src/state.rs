//! What every route shares: the canvas under its lock, the event sink and log, the terminal echo
//! and the places the host reads files from.
//!
//! It owns the lock discipline in one place. A route takes the canvas lock briefly and never across
//! an `.await`, and queues new audit lines for the terminal only after the lock is released (the
//! echo never waits for the terminal). It also owns what is shared between readers: the frame,
//! built once for each change (the canvas half, off the runtime threads) and handed to every page
//! as the same bytes, and the signal that wakes the host's clock when something starts to move. It
//! decides nothing about requests: `server` and `playground` own their routes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use tokio::sync::{Notify, watch};

use df_table::canvas::Canvas;
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;
use df_types::saturating_millis;

use crate::cast::routes::SpeakerRest;
use crate::creation::HeroSheetSummary;
use crate::engine::EngineHandle;
use crate::engine::report::EngineError;
use crate::engine::setup::EngineStartError;
use crate::event_log::EventLog;
use crate::limits::{TICK_INTERVAL, TURN_SETTLE};
use crate::observe::HostEventSink;
use crate::playground::{FileError, PlaygroundFiles, PlaygroundState};
use crate::seat::{PLAYERS_DIRECTORY, SeatDesk};
use crate::server::request::{ClockStatus, HostStatus, LogStatus};
use crate::sink::LineEcho;
use crate::view::{ViewError, frame_canvas};
use df_playground::controls::ControlsReply;
use df_playground::frame::{FrameCanvas, player_controls};
use df_playground::overlay::Edit;

/// A frame as the pieces of its JSON: the canvas half is built once for a change count and shared
/// by every reader of it; the head (the count) and the tail (what the playground shows, with each
/// move's age as of now) are made for each reader, because a move's age grows while the canvas
/// does not change.
#[derive(Debug, Clone)]
pub(crate) struct BuiltFrame {
    pub(crate) canvas: Arc<[u8]>,
    count: u64,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl BuiltFrame {
    /// The change count the frame was built at (its `frame` value).
    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// The JSON of the whole frame as the pieces in order, without copying the canvas.
    pub(crate) fn chunks(&self) -> [Bytes; 3] {
        [
            Bytes::copy_from_slice(&self.head),
            Bytes::from_owner(Arc::clone(&self.canvas)),
            Bytes::copy_from_slice(&self.tail),
        ]
    }

    /// The JSON of the whole frame in one piece, for a test.
    #[cfg(test)]
    pub(crate) fn to_vec(&self) -> Vec<u8> {
        [self.head.as_slice(), &self.canvas, self.tail.as_slice()].concat()
    }
}

/// The sheets of the party strip's heroes, read at one change count.
#[derive(Debug)]
struct PartySheets {
    count: u64,
    sheets: BTreeMap<String, PartyHero>,
}

/// The characters the party's cards name.
fn party_characters(party: &[df_playground::fixture::PartyMember]) -> BTreeSet<String> {
    party
        .iter()
        .filter_map(|member| member.character.clone())
        .collect()
}

/// What a party card's hero has: its sheet's numbers and its pack's headshot, each once made.
#[derive(Debug, Clone, PartialEq)]
struct PartyHero {
    sheet: Option<HeroSheetSummary>,
    headshot: Option<String>,
}

/// Who a frame is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameAudience {
    /// The shared screen and the developer's pages: all of it.
    Shared,
    /// A phone on the player listener: what a phone draws, nothing of another seat's.
    Player,
}

/// The canvas half of the frame for one change count.
#[derive(Debug, Clone)]
struct CachedCanvas {
    count: u64,
    json: Arc<[u8]>,
}

/// Why a frame could not be built.
#[derive(Debug)]
pub(crate) enum FrameError {
    View(ViewError),
    Json(serde_json::Error),
    /// The thread that builds the canvas half did not finish.
    Task(tokio::task::JoinError),
    /// The playground's files could not be read for their revision.
    Files(FileError),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::View(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "the frame could not be written: {error}"),
            Self::Task(error) => write!(formatter, "the frame could not be built: {error}"),
            Self::Files(error) => write!(formatter, "the playground's files: {error}"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::View(_) => None,
            Self::Json(error) => Some(error),
            Self::Task(error) => Some(error),
            Self::Files(error) => Some(error),
        }
    }
}

/// A name for this run of the host: the time it started and the process, so two runs never share
/// one and a page can tell a restarted host from the same one and an older run from a newer. A
/// clock before 1970 reads as zero,
/// and the process number still tells the runs apart.
pub(crate) fn new_boot_id() -> String {
    let nanoseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    static STARTED_IN_THIS_PROCESS: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let ordinal = STARTED_IN_THIS_PROCESS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Fixed width, so ids sort in the order the runs started and a page can tell an older run.
    format!(
        "{nanoseconds:032x}-{:08x}-{ordinal:08x}",
        std::process::id()
    )
}

/// Everything the handlers share.
pub struct AppState {
    canvas: Mutex<Canvas>,
    sink: HostEventSink,
    echo: Option<LineEcho>,
    kind_directory: PathBuf,
    playground: PlaygroundState,
    /// Counts the host's changes. A reader waiting for news waits on this.
    changes: watch::Sender<u64>,
    /// The canvas half of the frame last built, for the change count it was built at.
    frame: Mutex<Option<CachedCanvas>>,
    /// Held while the canvas half is built, so readers that miss the cache at the same moment wait
    /// for one build instead of each making their own.
    building: tokio::sync::Mutex<()>,
    /// When each element's turn last changed, for as long as the page is still gliding it.
    turns: Mutex<BTreeMap<ElementId, Instant>>,
    /// Signalled when an element starts to travel, so the clock can sleep while nothing moves.
    motion: Notify,
    /// How many ticks took the canvas lock, so a test can see that an idle clock does not.
    ticks: AtomicU64,
    /// The change count of the newest change that was not the clock moving something, so a frame
    /// line is written for what a person changed and not for each step of a travel.
    last_content_change: AtomicU64,
    /// One more than the newest `last_content_change` a `FRAME served` line was written for (0:
    /// none), so each change is logged once however many pages read it.
    frame_logged: AtomicU64,
    /// The revision of the playground's files (`playground::revision`) the frames carry, once read;
    /// the files watch (`refresh_files_revision`) keeps it current.
    files_revision: Mutex<Option<String>>,
    /// The sheets of the party strip's heroes as of a change count (`party_with_sheets`), so
    /// the creation record is read once a change, not once a page and a poll.
    party_sheets: tokio::sync::Mutex<Option<PartySheets>>,
    /// The pending pull-back of the camera after the last spoken line (`cast::routes`).
    speaker_rest: SpeakerRest,
    /// The pending clearing of the dungeon master's box after its last line (`persona narrate`),
    /// one rest timer like the camera's.
    narration_rest: SpeakerRest,
    /// The pending fade of the party strip after the party reached a new place
    /// (`world::routes::show_party_strip`).
    party_strip_rest: SpeakerRest,
    /// The phone page's addresses on the player listener, set once when it listens; unset while
    /// the host serves no players.
    player_links: OnceLock<Vec<String>>,
    /// The table's engine, set once when its routes start, so the phone's seat routes on either
    /// listener reach the same owner.
    engine: OnceLock<SharedEngine>,
    /// The players books: who holds which seat, the friend profiles and the heroes.
    seats: SeatDesk,
    /// The deals open between characters, until accepted, turned down or left behind.
    deals: crate::world::OpenDeals,
    /// The signer of the media links phones are given.
    media_links: crate::media_links::MediaLinks,
    /// The speech service, once the routes are built: a seat's own recordings are written down
    /// through it.
    speech: OnceLock<crate::speech::Speech>,
    /// The look studio, set once when the services start, so the phone's look routes on either
    /// listener reach the same picture generator and text service.
    look: OnceLock<Arc<crate::look::LookStudio>>,
    /// The check runner the persona routes started, so a seat's roll on either listener reaches
    /// the same judge, dice and table.
    check_rolls: OnceLock<Arc<crate::persona::check::CheckRoutes>>,
}

/// The engine the routes started, or why it did not start.
pub(crate) type SharedEngine = Arc<Result<EngineHandle, EngineStartError>>;

impl AppState {
    /// `echo` prints new audit lines to the terminal; `None` keeps the log quiet.
    pub fn new(
        canvas: Canvas,
        sink: HostEventSink,
        echo: Option<LineEcho>,
        kind_directory: PathBuf,
        playground_files: PlaygroundFiles,
    ) -> Self {
        let seats = SeatDesk::new(playground_files.assets().join(PLAYERS_DIRECTORY));
        Self {
            canvas: Mutex::new(canvas),
            sink,
            echo,
            kind_directory,
            playground: PlaygroundState::new(playground_files, new_boot_id()),
            changes: watch::channel(0).0,
            frame: Mutex::new(None),
            building: tokio::sync::Mutex::new(()),
            turns: Mutex::new(BTreeMap::new()),
            motion: Notify::new(),
            ticks: AtomicU64::new(0),
            last_content_change: AtomicU64::new(0),
            frame_logged: AtomicU64::new(0),
            files_revision: Mutex::new(None),
            party_sheets: tokio::sync::Mutex::new(None),
            speaker_rest: SpeakerRest::default(),
            narration_rest: SpeakerRest::default(),
            party_strip_rest: SpeakerRest::default(),
            player_links: OnceLock::new(),
            engine: OnceLock::new(),
            seats,
            deals: crate::world::OpenDeals::default(),
            media_links: crate::media_links::MediaLinks::new(),
            speech: OnceLock::new(),
            look: OnceLock::new(),
            check_rolls: OnceLock::new(),
        }
    }

    /// Remembers the engine its routes started. A second call keeps the first: the host runs one
    /// engine.
    pub(crate) fn set_engine(&self, engine: SharedEngine) {
        // The first call wins; a second router over the same state would start a second engine
        // that no phone should reach.
        let _ = self.engine.set(engine);
    }

    /// The running engine, or `EngineError` saying why there is none.
    pub(crate) fn engine(&self) -> Result<&EngineHandle, EngineError> {
        match self.engine.get().map(AsRef::as_ref) {
            Some(Ok(engine)) => Ok(engine),
            Some(Err(error)) => Err(EngineError::NotStarted {
                cause: error.cause(),
                detail: error.to_string(),
            }),
            None => Err(EngineError::Stopped),
        }
    }

    /// The players books.
    /// The speech service, once the routes were built.
    pub(crate) fn speech(&self) -> Option<&crate::speech::Speech> {
        self.speech.get()
    }

    /// Keeps the speech service the routes were built with; the first one stands.
    pub(crate) fn set_speech(&self, speech: crate::speech::Speech) {
        let _first_stands = self.speech.set(speech);
    }

    pub(crate) fn media_links(&self) -> &crate::media_links::MediaLinks {
        &self.media_links
    }

    pub(crate) fn deals(&self) -> &crate::world::OpenDeals {
        &self.deals
    }

    pub(crate) fn seats(&self) -> &SeatDesk {
        &self.seats
    }

    /// Remembers the look studio the services started. A second call keeps the first: the host
    /// runs one picture generator.
    pub(crate) fn set_look(&self, look: Arc<crate::look::LookStudio>) {
        // The first call wins; a second services router over the same state would start a second
        // studio no phone should reach.
        let _ = self.look.set(look);
    }

    /// The look studio, once the services started it.
    pub(crate) fn look(&self) -> Option<&Arc<crate::look::LookStudio>> {
        self.look.get()
    }

    /// Remembers the check runner the persona routes started. A second call keeps the first: the
    /// host runs one table of rolls.
    pub(crate) fn set_check_rolls(&self, checks: Arc<crate::persona::check::CheckRoutes>) {
        // The first call wins; a second persona router over the same state would roll on dice no
        // phone should reach.
        let _ = self.check_rolls.set(checks);
    }

    /// The check runner, once the persona routes started it.
    pub(crate) fn check_rolls(&self) -> Option<&Arc<crate::persona::check::CheckRoutes>> {
        self.check_rolls.get()
    }

    /// Remembers the phone page's addresses once the player listener listens. A second call keeps
    /// the first addresses: the host has one player listener.
    pub(crate) fn set_player_links(&self, links: Vec<String>) {
        // The first call wins; there is no second listener whose links could be lost.
        let _ = self.player_links.set(links);
    }

    /// The phone page's addresses on the player listener; empty while the host serves no players.
    pub(crate) fn player_links(&self) -> &[String] {
        self.player_links.get().map_or(&[], Vec::as_slice)
    }

    /// The pending pull-back of the camera after the last spoken line.
    pub(crate) fn speaker_rest(&self) -> &SpeakerRest {
        &self.speaker_rest
    }

    /// The pending clearing of the dungeon master's box after its last line.
    pub(crate) fn narration_rest(&self) -> &SpeakerRest {
        &self.narration_rest
    }

    pub(crate) fn party_strip_rest(&self) -> &SpeakerRest {
        &self.party_strip_rest
    }

    /// Says that something a reader could see has changed. A reader waiting on `changes` wakes.
    pub(crate) fn announce_change(&self) {
        self.changes.send_modify(|count| {
            *count += 1;
            // Stored under the channel's lock, so it never goes back past a later change.
            self.last_content_change.store(*count, Ordering::Relaxed);
        });
    }

    /// Says that the clock moved something: readers wake as for any change, but no frame line is
    /// written for it (a travel steps every tick).
    fn announce_motion(&self) {
        self.changes.send_modify(|count| *count += 1);
    }

    /// The count of changes so far.
    pub(crate) fn change_count(&self) -> u64 {
        *self.changes.borrow()
    }

    /// A receiver that sees every later announcement.
    pub(crate) fn watch_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Wakes the clock. Call it after an effect that may have started a travel; a signal with
    /// nothing waiting is kept, so one made before the clock parks is not lost.
    pub(crate) fn wake_clock(&self) {
        self.motion.notify_one();
    }

    /// Waits until the clock is woken.
    pub(crate) async fn clock_woken(&self) {
        self.motion.notified().await;
    }

    /// Says that the turn of `element` just changed, so the page is gliding it for `TURN_SETTLE`.
    /// Elements whose glide is over are forgotten, so this holds only the ones turned lately.
    pub(crate) fn note_turn(&self, element: ElementId) {
        let now = Instant::now();
        let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        turns.retain(|_, turned| now.saturating_duration_since(*turned) < TURN_SETTLE);
        turns.insert(element, now);
    }

    /// How much longer the page is gliding the turn of `element`, or `None` when it is not.
    pub(crate) fn turn_settling(&self, element: &ElementId) -> Option<Duration> {
        let turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        let turned = turns.get(element)?;
        TURN_SETTLE.checked_sub(Instant::now().saturating_duration_since(*turned))
    }

    /// Forgets the turns of `elements` that no longer exist.
    pub(crate) fn forget_turns(&self, elements: &[String]) {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|element, _| !elements.iter().any(|gone| gone == element.as_str()));
    }

    /// Whether any element is travelling.
    pub(crate) fn is_moving(&self) -> bool {
        self.canvas().is_moving()
    }

    /// How many ticks have taken the canvas lock.
    pub fn tick_count(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// How many elements are travelling. It copies the elements under the canvas lock, so it is
    /// for the clock's wake-up line and the status, not for every tick.
    pub(crate) fn moving_count(&self) -> usize {
        self.canvas()
            .elements()
            .filter(|element| element.motion.is_some())
            .count()
    }

    /// `true` the first time a frame of change count `count` that holds the newest change that was
    /// not the clock's is served, so the caller writes its `FRAME served` line once per such
    /// change. Frames that differ only by the clock's steps of a travel are not logged.
    pub(crate) fn first_to_serve_frame(&self, count: u64) -> bool {
        let change = self.last_content_change.load(Ordering::Relaxed);
        if count < change {
            return false;
        }
        let marker = change.saturating_add(1);
        self.frame_logged.fetch_max(marker, Ordering::Relaxed) < marker
    }

    fn cached_canvas(&self, count: u64) -> Option<Arc<[u8]>> {
        self.frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|cached| cached.count == count)
            .map(|cached| Arc::clone(&cached.json))
    }

    /// The canvas half of the frame as JSON: every element is copied once under the canvas lock,
    /// and written after the lock is released. Blocking work: call it off the runtime threads.
    fn build_canvas_json(&self) -> Result<Arc<[u8]>, FrameError> {
        let canvas = frame_canvas(&self.canvas()).map_err(FrameError::View)?;
        let json = serde_json::to_vec(&canvas).map_err(FrameError::Json)?;
        Ok(json.into())
    }

    /// The shared screen's frame for the current change count (`frame_for`), for a test.
    #[cfg(test)]
    pub(crate) async fn frame(self: &Arc<Self>) -> Result<BuiltFrame, FrameError> {
        self.frame_for(FrameAudience::Shared).await
    }

    /// The frame for the current change count, as `audience` is sent it. The shared screen's
    /// canvas half is built once per count, on the blocking pool, and every reader of that count
    /// shares the same bytes; the playground half is made for each reader, so a move's
    /// `elapsed_ms` is its age now and a late reader continues from where the move is. A phone's
    /// frame leaves out what a phone does not draw (`df_playground::frame::Frame::for_player`).
    pub(crate) async fn frame_for(
        self: &Arc<Self>,
        audience: FrameAudience,
    ) -> Result<BuiltFrame, FrameError> {
        if audience == FrameAudience::Player {
            return self.player_frame().await;
        }
        let mut count = self.change_count();
        let canvas = match self.cached_canvas(count) {
            Some(canvas) => canvas,
            None => {
                let _building = self.building.lock().await;
                // Another reader may have built it while this one waited.
                count = self.change_count();
                match self.cached_canvas(count) {
                    Some(canvas) => canvas,
                    None => {
                        let builder = Arc::clone(self);
                        let json = tokio::task::spawn_blocking(move || builder.build_canvas_json())
                            .await
                            .map_err(FrameError::Task)??;
                        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(CachedCanvas {
                                count,
                                json: Arc::clone(&json),
                            });
                        json
                    }
                }
            }
        };
        let controls = self.party_with_sheets(self.playground.snapshot()).await;
        self.assemble_frame(count, canvas, &controls).await
    }

    /// A phone's frame: the canvas's revision without its elements, and the controls a phone
    /// draws (`df_playground::frame::player_controls`). Small, so it is made for each reader.
    async fn player_frame(&self) -> Result<BuiltFrame, FrameError> {
        let count = self.change_count();
        let canvas = FrameCanvas {
            revision: self.canvas().revision(),
            elements: Vec::new(),
        };
        let canvas = serde_json::to_vec(&canvas).map_err(FrameError::Json)?;
        let controls = player_controls(self.party_with_sheets(self.playground.snapshot()).await);
        self.assemble_frame(count, canvas.into(), &controls).await
    }

    /// The frame's JSON pieces around its canvas half: its count, its controls and the revision of
    /// the playground's files.
    async fn assemble_frame(
        &self,
        count: u64,
        canvas: Arc<[u8]>,
        controls: &ControlsReply,
    ) -> Result<BuiltFrame, FrameError> {
        let controls = serde_json::to_vec(controls).map_err(FrameError::Json)?;
        let files = serde_json::to_vec(&self.files_revision().await?).map_err(FrameError::Json)?;
        let head = format!("{{\"frame\":{count},\"canvas\":").into_bytes();
        let tail = [
            b",\"controls\":".as_slice(),
            controls.as_slice(),
            b",\"asset_revision\":",
            files.as_slice(),
            b"}",
        ]
        .concat();
        Ok(BuiltFrame {
            canvas,
            count,
            head,
            tail,
        })
    }

    /// The controls with the party strip's heroes' numbers from their sheets: a card that names
    /// its character (`PartyMember::character`) shows the hit points and Armor Class of that
    /// hero's sheet, and none until the hero has one (never a sample), and the headshot of the
    /// hero's pack as its picture once it is made. A card that names no character keeps what it
    /// was given.
    pub(crate) async fn party_with_sheets(&self, mut reply: ControlsReply) -> ControlsReply {
        let Edit::Set(party) = &mut reply.scene.party else {
            return reply;
        };
        let characters = party_characters(party);
        if characters.is_empty() {
            return reply;
        }
        let sheets = self.party_sheets(characters).await;
        for member in party.iter_mut() {
            let Some(character) = &member.character else {
                continue;
            };
            let Some(hero) = sheets.get(character) else {
                continue;
            };
            let sheet = hero.sheet;
            member.hit_points = sheet.map(|sheet| sheet.current_hp);
            member.maximum_hit_points = sheet.map(|sheet| sheet.max_hp);
            member.armor_class = sheet.map(|sheet| sheet.armor_class);
            if hero.headshot.is_some() {
                member.portrait.clone_from(&hero.headshot);
            }
        }
        reply
    }

    /// The sheets of `characters`, read from the creation record when the change count moved
    /// since they were last read (a sheet is made or changed through a change). A record that
    /// cannot be read gives no sheet and writes `PARTY sheet_unreadable character= error=`.
    async fn party_sheets(&self, characters: BTreeSet<String>) -> BTreeMap<String, PartyHero> {
        let count = self.change_count();
        let mut kept = self.party_sheets.lock().await;
        if let Some(sheets) = kept.as_ref().filter(|sheets| {
            sheets.count == count
                && characters
                    .iter()
                    .all(|character| sheets.sheets.contains_key(character))
        }) {
            return sheets.sheets.clone();
        }
        let sheets = self.read_party_heroes(characters).await;
        *kept = Some(PartySheets {
            count,
            sheets: sheets.clone(),
        });
        sheets
    }

    /// The party strip's heroes now: what the cards name, read again. A hero's headshot is made
    /// in the background and a sheet can change with no change of the table, so the files watch
    /// asks this: a difference is kept and announced as a change, and the cards follow within its
    /// interval. Whether it changed.
    pub(crate) async fn refresh_party_heroes(&self) -> bool {
        let reply = self.playground.snapshot();
        let Edit::Set(party) = &reply.scene.party else {
            return false;
        };
        let characters = party_characters(party);
        if characters.is_empty() {
            return false;
        }
        let fresh = self.read_party_heroes(characters).await;
        let unchanged = self
            .party_sheets
            .lock()
            .await
            .as_ref()
            .is_some_and(|kept| kept.sheets == fresh);
        if unchanged {
            return false;
        }
        self.announce_change();
        *self.party_sheets.lock().await = Some(PartySheets {
            count: self.change_count(),
            sheets: fresh,
        });
        true
    }

    /// Reads the sheet and the headshot of each of `characters`.
    async fn read_party_heroes(&self, characters: BTreeSet<String>) -> BTreeMap<String, PartyHero> {
        let mut sheets = BTreeMap::new();
        for character in characters {
            let sheet = match crate::creation::hero_summary(self, &character).await {
                Ok(sheet) => sheet,
                Err(error) => {
                    self.record_line(format!(
                        "PARTY sheet_unreadable character={character} error={}",
                        error.key()
                    ));
                    None
                }
            };
            let headshot = self.headshot_of(&character).await;
            sheets.insert(character, PartyHero { sheet, headshot });
        }
        sheets
    }

    /// The headshot of the pack of the hero whose character is `character` (a seat's hero, as its
    /// persona names it), once it is made.
    async fn headshot_of(&self, character: &str) -> Option<String> {
        let mut open = self.seats().open().await.ok()?;
        let books = open.books();
        let hero = books
            .records()
            .iter()
            .filter_map(|record| record.hero)
            .find(|hero| crate::seat::scenario::persona_of(*hero) == character)?;
        books.hero(hero)?.pack.headshot.picture().map(str::to_owned)
    }

    /// The revision of the playground's files the frames carry: the one kept, or the files read
    /// now (off the runtime threads) when none is kept yet.
    async fn files_revision(&self) -> Result<String, FrameError> {
        let kept = self
            .files_revision
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match kept {
            Some(revision) => Ok(revision),
            None => {
                let revision = self
                    .read_files_revision()
                    .await
                    .map_err(FrameError::Files)?;
                *self
                    .files_revision
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(revision.clone());
                Ok(revision)
            }
        }
    }

    async fn read_files_revision(&self) -> Result<String, FileError> {
        let files = self.playground.files().clone();
        crate::blocking::run_blocking(
            move || crate::playground::revision(&files),
            FileError::Stopped,
        )
        .await
    }

    /// Reads the playground's files again: a revision other than the one kept is kept and
    /// announced as a change, so every page learns of a new theme, fixture or package from its
    /// next frame. The new revision, when it changed. One read serves every page, however many
    /// follow the host.
    pub(crate) async fn refresh_files_revision(&self) -> Result<Option<String>, FileError> {
        let revision = self.read_files_revision().await?;
        let changed = {
            let mut kept = self
                .files_revision
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let changed = kept.as_ref().is_some_and(|kept| *kept != revision);
            *kept = Some(revision.clone());
            changed
        };
        if !changed {
            return Ok(None);
        }
        self.announce_change();
        Ok(Some(revision))
    }

    /// Advances every travelling element by `seconds` of the host's clock. Idle, it takes the lock
    /// and returns. A tick that moved something is announced, and an arrival is echoed.
    pub fn tick(&self, seconds: f32) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        let report = {
            let mut canvas = self.canvas();
            if !canvas.is_moving() {
                return;
            }
            canvas.tick(seconds, &self.sink)
        };
        if report.moved > 0 || !report.arrived.is_empty() {
            self.announce_motion();
        }
        if !report.arrived.is_empty() {
            self.echo_new_lines();
        }
    }

    pub(crate) fn canvas(&self) -> MutexGuard<'_, Canvas> {
        self.canvas.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn sink(&self) -> &HostEventSink {
        &self.sink
    }

    pub(crate) fn log(&self) -> &Arc<EventLog> {
        self.sink.log()
    }

    pub(crate) fn kind_directory(&self) -> &Path {
        &self.kind_directory
    }

    pub(crate) fn playground(&self) -> &PlaygroundState {
        &self.playground
    }

    /// Records a rejection that happened before a request became an effect.
    pub(crate) fn record_rejection(
        &self,
        effect: Option<EffectKind>,
        element: Option<ElementId>,
        cause: RejectionCause,
    ) {
        self.sink.record(&TableEvent::EffectRejected {
            effect,
            element,
            cause,
        });
    }

    /// Records one audit line that is not a table event, such as a playground change. Inside a
    /// request this is all a route does: the `guard_requests` middleware echoes the request's lines
    /// once it has been answered. It takes no lock but the log's, so it may run under the canvas
    /// lock.
    pub(crate) fn record_line(&self, text: String) {
        self.log().push(text);
    }

    /// Records one audit line and queues it for the terminal at once, for work that goes on after
    /// its request was answered (a background job), which no middleware echoes. Call it with no
    /// lock held.
    pub(crate) fn record_line_and_echo(&self, text: String) {
        self.record_line(text);
        self.echo_new_lines();
    }

    /// Waits up to `patience` for the terminal echo to print what is queued, and stops it taking
    /// more. `true` when it did (or there is no echo).
    pub fn finish_echo(&self, patience: Duration) -> bool {
        self.echo.as_ref().is_none_or(|echo| echo.finish(patience))
    }

    /// The log and its echo, as `host status` shows them.
    pub(crate) fn log_status(&self) -> LogStatus {
        let counts = self.log().counts();
        LogStatus {
            latest: counts.latest,
            kept: counts.kept,
            bytes: counts.bytes,
            forgotten: counts.forgotten,
            echo_pending: self.echo.as_ref().map_or(0, LineEcho::pending_bytes),
            echo_dropped: self.echo.as_ref().map_or(0, LineEcho::dropped_lines),
            echo_writer_gone: self.echo.as_ref().is_some_and(LineEcho::writer_gone),
            undelivered_audit_lines: df_observe::undelivered_audit_lines(),
        }
    }

    /// What `GET /dev/host/status` answers.
    pub(crate) fn host_status(&self) -> HostStatus {
        HostStatus {
            log: self.log_status(),
            sink: self.sink.health(),
            clock: ClockStatus {
                ticks: self.tick_count(),
                moving: self.moving_count(),
                tick_ms: saturating_millis(TICK_INTERVAL),
            },
            engine_sink: None,
            player_links: self.player_links().to_vec(),
        }
    }

    /// Prints new audit lines. Call it with no lock held.
    pub(crate) fn echo_new_lines(&self) {
        if let Some(echo) = &self.echo {
            echo.echo_new(self.log());
        }
    }
}
