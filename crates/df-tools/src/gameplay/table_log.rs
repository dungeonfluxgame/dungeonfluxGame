//! The public table log: the latest lines of who said and did what, shown to every screen.
//!
//! Everything here is table talk: an accepted action's public phrase, the words a hero types to
//! the scene's speaker, and the speaker's in-character reply. The private clue and any other
//! member-private text are never written here by any caller. The log is a bounded in-memory ring
//! per run; it is presentation only and never an authority for game state.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use tokio::sync::watch;

/// The speaker of the server's own notes in the log (not story, so the story memory skips them).
pub(super) const SYSTEM_SPEAKER: &str = "table";
/// The speaker of a check's line, written by [`TableLog::push_check`].
pub(super) const CHECK_SPEAKER: &str = "Roll";

/// One roll row: the text the table reads and, when the server knows the numbers, the structured
/// roll the shared display plays as a die (never parsed back out of the text).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RollRow {
    pub(super) text: String,
    pub(super) roll: Option<rpc::TableRoll>,
}

/// The short label a roll's result card shows: "Brakka · Deception" from the subject and the
/// check's label ("Charisma (Deception, Advantage: 8 and 1)"). The skill is the parenthesised
/// name up to its first comma, else the whole label ("Dexterity save").
pub(super) fn roll_label(subject: &str, label: &str) -> String {
    let skill = match label.split_once('(') {
        Some((_, rest)) => rest
            .split([',', ')'])
            .next()
            .unwrap_or(rest)
            .trim()
            .to_owned(),
        None => label.trim().to_owned(),
    };
    if skill.is_empty() {
        subject.trim().to_owned()
    } else {
        format!("{} \u{b7} {skill}", subject.trim())
    }
}

/// The display name of a skill id as the check receipts carry it ("sleight-of-hand").
fn skill_display(id: &str) -> Option<&'static str> {
    Some(match id {
        "perception" => "Perception",
        "insight" => "Insight",
        "survival" => "Survival",
        "medicine" => "Medicine",
        "investigation" => "Investigation",
        "arcana" => "Arcana",
        "history" => "History",
        "nature" => "Nature",
        "religion" => "Religion",
        "stealth" => "Stealth",
        "athletics" => "Athletics",
        "acrobatics" => "Acrobatics",
        "sleight-of-hand" => "Sleight of Hand",
        "persuasion" => "Persuasion",
        "deception" => "Deception",
        "intimidation" => "Intimidation",
        "performance" => "Performance",
        _ => return None,
    })
}

/// The roll row of an accepted ability check receipt: `text` is the row's words, and the die, the
/// modifier, the total, the difficulty class and the verdict come from the receipt itself. The
/// receipt's source revision (`..:..:<skill id>:<detail>`) names the skill for the short label.
pub(super) fn ability_check_row(
    text: String,
    hero: &str,
    check: &rpc::AbilityCheckResult,
) -> RollRow {
    let skill = check
        .source_revision
        .splitn(4, ':')
        .nth(2)
        .and_then(skill_display);
    let faces = if check.faces.is_empty() {
        vec![check.die]
    } else {
        check.faces.clone()
    };
    RollRow {
        text,
        roll: Some(rpc::TableRoll {
            faces,
            kept: check.die,
            roll_mode: check.roll_mode,
            modifier: check
                .ability_modifier
                .saturating_add(i32::try_from(check.proficiency_bonus).unwrap_or_default()),
            total: check.total,
            against: rpc::RollAgainst::Dc as i32,
            target_number: check.difficulty_class,
            succeeded: check.succeeded,
            label: roll_label(hero, skill.map_or("", |skill| skill)),
            kind: rpc::RollKind::Check as i32,
        }),
    }
}

impl From<&str> for RollRow {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            roll: None,
        }
    }
}

impl From<&String> for RollRow {
    fn from(text: &String) -> Self {
        text.as_str().into()
    }
}

impl From<String> for RollRow {
    fn from(text: String) -> Self {
        Self { text, roll: None }
    }
}

/// Receives a copy of every public line as it is appended, and the floor of a rewind. The story
/// memory's tap: it must not block, because it runs on whichever thread appended the line.
pub(super) trait LineSink: Send + Sync {
    fn line(&self, line: &rpc::TableLine);
    fn rewound(&self, floor: u64);
}

/// How many of the latest lines a view carries.
pub(super) const RING_LINES: usize = 24;
/// Longest text of one line, in characters.
pub(super) const LINE_CHARACTERS: usize = 420;
/// A speech-intent mark (typing or talking) that is not refreshed this long is gone, so a
/// closed tab never leaves a stuck bubble on the table. The phone refreshes about every 2 s.
pub(super) const PRESENCE_EXPIRY: Duration = Duration::from_secs(5);
/// Hero seats a presence mark can name.
const PRESENCE_HEROES: u32 = 2;

/// The longest one commit may hold the phones without a display report, however many
/// beats it resolved: a missing or frozen display can never block play past this
/// bound. Per-beat fallback durations come from the one shared table the display's
/// stage reads (`df_render::battle::beat_timing`); the display's report is the primary
/// release signal and the timer only bounds its absence.
const PACING_MAX_WAIT_MS: u64 = 20_000;
/// A display whose newest report is older than this no longer governs the release: a
/// closed, frozen or hidden-away display stops holding the table after this bound,
/// and the pacing timer wakes then to drop it even if no report ever arrives again.
/// A live display re-reports about every 1.6 s while the server is behind it, so this
/// bound is more than two missed reports.
const ACK_STALE_MS: u64 = 4_000;
/// How long the phones' story choices stay withheld after the scene changes outside a fight
/// (FLO-01): long enough for the new scene to be on the TV before anyone can leave it. The TV
/// does not report when a scene has been told, so this is a fixed, bounded wait.
const STORY_HOLD_MS: u64 = 4_000;
/// Displays whose reports are tracked at once; beyond this the stalest report is
/// dropped. Several televisions on one room stay distinguishable by instance.
const ACK_DISPLAYS: usize = 8;

fn visible_table_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LINE_CHARACTERS)
        .collect()
}

/// The server-side presentation gate, as one typed state so "phones may act on beat N"
/// and "beat N is still being shown" cannot both be recorded. Transitions happen only in
/// [`TableLog::pacing_observe`] and [`TableLog::expire_pacing_at`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pacing {
    /// Every resolved beat has been released: phones may be offered the next action.
    CaughtUp { released: u64 },
    /// Beats `released + 1..=latest` are being presented; offers wait until `release_at`.
    Presenting {
        released: u64,
        latest: u64,
        stamped_at: Instant,
        release_at: Instant,
    },
}

impl Pacing {
    fn released(self) -> u64 {
        match self {
            Self::CaughtUp { released } => released,
            Self::Presenting { released, .. } => released,
        }
    }

    /// The newest beat this state knows about; a view below it belongs to a new fight.
    fn latest_known(self) -> u64 {
        match self {
            Self::CaughtUp { released } => released,
            Self::Presenting { latest, .. } => latest,
        }
    }
}

/// How long one resolved outcome takes on the display, by kind, from the shared
/// beat-timing table the display's stage plans from. A move is priced by its squares.
fn beat_duration_ms(outcome: &rpc::CombatOutcome) -> u64 {
    use df_render::battle::beat_timing;
    match rpc::OutcomeKind::try_from(outcome.kind) {
        Ok(rpc::OutcomeKind::Move) => {
            beat_timing::move_ms(u32::try_from(outcome.path.len()).unwrap_or(1))
        }
        Ok(rpc::OutcomeKind::DeathSave | rpc::OutcomeKind::Recovery) => {
            beat_timing::death_save_ms()
        }
        _ => beat_timing::blow_ms(
            outcome.attack_style == 2,
            outcome.hit || outcome.grazed,
            outcome.knocked_out,
        ),
    }
}

/// One hero's live speech intent. Ephemeral: never stored in the game, never a revision.
struct Composing {
    hero_index: u32,
    kind: rpc::ComposeKind,
    deadline: Instant,
}

struct Ring {
    next_sequence: u64,
    lines: VecDeque<rpc::TableLine>,
    /// Counts every change a view shows (a line appended, a reply starting or ending), so a
    /// view whose game revision did not move can still be told from the one before it.
    version: u64,
    /// Counts speech-intent changes, which only the shared display shows: a phone's view (and
    /// so its mounted controls) is never rebuilt because another player is typing.
    presence_version: u64,
    /// The speakers whose reply to a hero's typed line is being written right now.
    replying: Vec<String>,
    /// Heroes typing or talking right now, until stopped or expired.
    composing: Vec<Composing>,
    /// A task is waiting to expire the earliest mark.
    reaper_armed: bool,
    /// The fight presentation gate: which resolved beats the phones may act past.
    pacing: Pacing,
    /// The instant the live pacing timer task will wake at, when one is running. The
    /// task itself owns clearing this (inside the same lock as the release it makes),
    /// so a newer wait can never find the flag set by a task that has already gone.
    pacing_armed_until: Option<Instant>,
    /// The pacing timer could not be armed (no runtime handle captured yet) and the
    /// failure has been logged; cleared by the next successful arm so a later failure
    /// logs again instead of stalling silently.
    pacing_arm_failed: bool,
    /// Until when the phones' story choices are withheld; see [`STORY_HOLD_MS`].
    story_hold_until: Option<Instant>,
    /// The story scenes the phones were last shown (phase, title, place), newest last. A view
    /// of a scene not listed here starts the hold; two are kept so that two views that differ
    /// for any reason can never restart it against each other.
    story_scenes: Vec<(i32, String, String)>,
    /// The newest presentation report of each display instance, `(sequence, at)`. The
    /// slowest display whose report is still fresh governs the release; a display
    /// that stops reporting is dropped after [`ACK_STALE_MS`] and stops governing.
    acks: BTreeMap<u64, (u64, Instant)>,
}

pub(super) struct TableLog {
    /// A disabled log (qualification mode) records nothing and attaches nothing.
    enabled: bool,
    /// How long a speech-intent mark lives without a refresh.
    expiry: Duration,
    ring: Mutex<Ring>,
    /// Carries the latest sequence so a watcher rebuilds its view when a line is appended.
    wake: watch::Sender<u64>,
    /// The async runtime's handle, captured from whichever call first runs inside it (a
    /// watch subscription). View publishing runs on the session owner thread, which has no
    /// runtime of its own: without the captured handle the expiry/pacing timers could never
    /// be armed from there and a wait would never end on its own.
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    /// The tap that copies each appended line to the story memory, once installed.
    sink: OnceLock<Arc<dyn LineSink>>,
}

/// A sequence reserved for an accepted action line. Dropping it leaves an empty sequence gap.
pub(super) struct TableLogReservation {
    sequence: Option<u64>,
}

impl TableLog {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::with(true))
    }

    pub(super) fn disabled() -> Arc<Self> {
        Arc::new(Self::with(false))
    }

    fn with(enabled: bool) -> Self {
        Self {
            enabled,
            expiry: PRESENCE_EXPIRY,
            ring: Mutex::new(Ring {
                next_sequence: 1,
                lines: VecDeque::new(),
                version: 0,
                presence_version: 0,
                replying: Vec::new(),
                composing: Vec::new(),
                reaper_armed: false,
                pacing: Pacing::CaughtUp { released: 0 },
                pacing_armed_until: None,
                pacing_arm_failed: false,
                story_hold_until: None,
                story_scenes: Vec::new(),
                acks: BTreeMap::new(),
            }),
            wake: watch::channel(0).0,
            // Construction usually runs inside the server's runtime; capture its handle
            // now so the first timer can arm even if it is requested (on the session
            // owner thread) before any watcher subscribes.
            runtime: Mutex::new(tokio::runtime::Handle::try_current().ok()),
            sink: OnceLock::new(),
        }
    }

    /// The runtime to spawn timer tasks on: the caller's when it has one (captured for
    /// later owner-thread calls), else the captured one.
    fn runtime(&self) -> Option<tokio::runtime::Handle> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            *self.runtime.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle.clone());
            return Some(handle);
        }
        self.runtime
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Subscribes to appended lines; a change means "publish a fresh view". A
    /// subscription runs inside the async runtime, so it also captures the runtime
    /// handle and re-arms a pacing wait whose timer could not be armed earlier (the
    /// wait was entered on the owner thread before any watcher arrived): arming can
    /// never be lost just because of the thread it was first requested on.
    pub(super) fn subscribe(self: &Arc<Self>) -> watch::Receiver<u64> {
        let _captured = self.runtime();
        if let Some(at) = self.pacing_next_wake() {
            self.arm_pacing(at);
        }
        self.wake.subscribe()
    }

    /// The instant the pacing timer should next wake at, when a wait is in progress:
    /// the earlier of the release bound and the moment the stalest governing report
    /// ages out.
    fn pacing_next_wake(&self) -> Option<Instant> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        Self::next_wake_locked(&ring)
    }

    fn next_wake_locked(ring: &Ring) -> Option<Instant> {
        let beats = match ring.pacing {
            Pacing::Presenting { release_at, .. } => {
                let stale = Duration::from_millis(ACK_STALE_MS);
                let ages_out = ring.acks.values().map(|(_, at)| *at + stale).min();
                Some(ages_out.map_or(release_at, |at| at.min(release_at)))
            }
            Pacing::CaughtUp { .. } => None,
        };
        match (beats, ring.story_hold_until) {
            (Some(beats), Some(story)) => Some(beats.min(story)),
            (beats, story) => beats.or(story),
        }
    }

    /// Ends the story hold when its time has come. Returns whether it ended (watchers must be
    /// woken so the phones are sent their choices).
    fn release_story_locked(ring: &mut Ring, now: Instant) -> bool {
        if ring.story_hold_until.is_some_and(|until| now >= until) {
            ring.story_hold_until = None;
            println!("STORY release cause=timeout");
            return true;
        }
        false
    }

    /// Advances the gate as of `now` under the caller's lock: drops reports whose
    /// display has gone silent past the bound, lets the slowest fresh report advance
    /// the released sequence, and lands the timer release when its time has come.
    /// Returns whether the released sequence moved (watchers must be woken).
    fn advance_pacing_locked(ring: &mut Ring, now: Instant) -> bool {
        let Pacing::Presenting {
            released,
            latest,
            stamped_at,
            release_at,
        } = ring.pacing
        else {
            return false;
        };
        let stale = Duration::from_millis(ACK_STALE_MS);
        ring.acks
            .retain(|_, (_, at)| now.saturating_duration_since(*at) <= stale);
        if now >= release_at {
            ring.pacing = Pacing::CaughtUp { released: latest };
            ring.acks.clear();
            return true;
        }
        let Some(governing) = ring.acks.values().map(|(seq, _)| *seq).min() else {
            return false;
        };
        let governing = governing.clamp(released, latest);
        if governing <= released {
            return false;
        }
        if governing >= latest {
            ring.pacing = Pacing::CaughtUp { released: latest };
            ring.acks.clear();
        } else {
            ring.pacing = Pacing::Presenting {
                released: governing,
                latest,
                stamped_at,
                release_at,
            };
        }
        true
    }

    /// Installs the tap that copies every appended public line (once; a second call is ignored).
    pub(super) fn set_sink(&self, sink: Arc<dyn LineSink>) {
        let _already = self.sink.set(sink);
    }

    /// The first table sequence not yet assigned: the floor a development rewind
    /// (DEVCTL-001) truncates back to.
    pub(super) fn position(&self) -> u64 {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_sequence
    }

    /// Rewinds the public log to a floor taken by [`Self::position`]: drops every
    /// line from the future, resets the sequence counter, clears stale display
    /// presentation reports and releases the fight pacing gate, then wakes every
    /// screen so each one shows the restored moment. Sequence floors are exact
    /// even when the bounded line ring has wrapped, because sequences never
    /// repeat within one owner lifetime.
    pub(super) fn rewind_to(&self, floor: u64) {
        if !self.enabled {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            ring.lines.retain(|line| line.sequence < floor);
            ring.next_sequence = floor.min(ring.next_sequence);
            ring.pacing = Pacing::CaughtUp { released: 0 };
            ring.acks.clear();
            ring.story_hold_until = None;
            ring.version += 1;
            ring.version + ring.presence_version
        };
        self.wake.send_replace(version);
        if let Some(sink) = self.sink.get() {
            sink.rewound(floor);
        }
    }

    /// Reserves the next sequences for the narration lines of an act that is about to be
    /// submitted (a fight's start, told by its caller off the actor thread): the lines fill the
    /// reserved sequences afterwards, so they stand on the table before the combat rows the
    /// submission itself publishes. Dropping a reservation leaves only an empty gap.
    pub(super) fn reserve_lines(&self, count: usize) -> Vec<TableLogReservation> {
        (0..count).map(|_| self.reserve()).collect()
    }

    /// Reserves the next sequence before a durable action can publish its checkpoint. If the
    /// action is rejected, replayed or fails, dropping the reservation leaves only a gap.
    pub(super) fn reserve(&self) -> TableLogReservation {
        if !self.enabled {
            return TableLogReservation { sequence: None };
        }
        let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let sequence = ring.next_sequence;
        ring.next_sequence += 1;
        TableLogReservation {
            sequence: Some(sequence),
        }
    }

    /// A table setting changed, or a notebook entry was written: every screen gets one fresh view,
    /// though no line was said and the game revision did not move. The version it bumps is the one
    /// each view carries.
    pub(super) fn settings_changed(&self) {
        if !self.enabled {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            ring.version += 1;
            ring.version
        };
        self.wake.send_replace(version);
    }

    /// Appends one line. Control characters become spaces, the text is cut to
    /// [`LINE_CHARACTERS`] characters and an empty line is dropped.
    pub(super) fn push(
        &self,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> Option<u64> {
        self.push_line(None, (speaker, ""), role, hero_index, kind, (text, None))
    }

    /// Appends one public row for each non-empty line of a short narration.
    pub(super) fn push_lines(
        &self,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> Vec<Option<u64>> {
        text.lines()
            .filter(|line| !line.is_empty())
            .map(|line| self.push(speaker, role, hero_index, kind, line))
            .collect()
    }

    pub(super) fn push_actor(
        &self,
        actor_id: &str,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> Option<u64> {
        self.push_line(
            None,
            (speaker, actor_id),
            role,
            hero_index,
            kind,
            (text, None),
        )
    }

    pub(super) fn push_speech(&self, speaker: &str, hero_index: u32, text: &str) {
        self.push(
            speaker,
            rpc::TableRole::Hero,
            hero_index,
            rpc::TableKind::Speech,
            text,
        );
    }

    /// Fills an action's earlier reserved sequence; subsequent lines use [`Self::push`].
    pub(super) fn push_reserved(
        &self,
        reservation: TableLogReservation,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> Option<u64> {
        self.push_line(
            reservation.sequence,
            (speaker, ""),
            role,
            hero_index,
            kind,
            (text, None),
        )
    }

    /// Appends one line an accepted action produced, with the roll the display plays for it.
    /// The first line of an action fills the sequence the action reserved when it was
    /// submitted; the others take the next sequences.
    pub(super) fn push_action_line(
        &self,
        reservation: Option<TableLogReservation>,
        line: &ActionLine,
        role: rpc::TableRole,
        hero_index: u32,
    ) {
        self.push_line(
            reservation.and_then(|reservation| reservation.sequence),
            (&line.speaker, ""),
            role,
            hero_index,
            rpc::TableKind::Action,
            (&line.text, line.roll.clone()),
        );
    }

    /// A scene interaction's rows, in table order: its roll row when it rolled one, then the
    /// narrator's outcome. The reservation is the one the action took when it was submitted.
    pub(super) fn push_scene_outcome(
        &self,
        reservation: TableLogReservation,
        roller_index: u32,
        check: Option<impl Into<RollRow>>,
        outcome: &str,
    ) {
        match check {
            Some(check) => {
                self.push_check(reservation, roller_index, check);
                self.push(
                    "Narrator",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    outcome,
                );
            }
            None => {
                self.push_reserved(
                    reservation,
                    "Narrator",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    outcome,
                );
            }
        }
    }

    /// A check is rules narration; the hero's name and outcome belong in its text. A row that
    /// carries its structured roll lets the shared display play the die for it.
    pub(super) fn push_check(
        &self,
        reservation: TableLogReservation,
        roller_index: u32,
        row: impl Into<RollRow>,
    ) {
        let row = row.into();
        self.push_line(
            reservation.sequence,
            (CHECK_SPEAKER, ""),
            rpc::TableRole::Narrator,
            roller_index,
            rpc::TableKind::Action,
            (&row.text, row.roll),
        );
    }

    fn push_line(
        &self,
        reserved_sequence: Option<u64>,
        speaker: (&str, &str),
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        body: (&str, Option<rpc::TableRoll>),
    ) -> Option<u64> {
        let (speaker, actor_id) = speaker;
        let (text, roll) = body;
        if let Some((entry_kind, title, scene)) = notebook_entry_log_fields(text) {
            let context = df_observe::OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&context, "gameplay.notebook_entry");
            span.event(
                "notebook.entry_added",
                &[
                    ("kind", entry_kind.to_owned()),
                    ("title", title.to_owned()),
                    ("scene", scene.to_owned()),
                ],
            );
            span.finish_unmeasured("recorded");
            // A notebook write is not a table line, but the phone's notebook rides in the view,
            // and a view at an unchanged revision is dropped as a duplicate. Wake the watchers so
            // the next view carries the entry.
            self.settings_changed();
            return None;
        }
        if !self.enabled {
            return None;
        }
        let text = visible_table_line(text);
        if text.is_empty() {
            return None;
        }
        super::actor::record_audit_line(
            "gameplay.table_line",
            format!("LINE queued speaker={speaker:?} role={role:?} kind={kind:?} text={text:?}"),
        );
        let (version, sequence, tapped) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let sequence = reserved_sequence.unwrap_or_else(|| {
                let sequence = ring.next_sequence;
                ring.next_sequence += 1;
                sequence
            });
            let position = ring.lines.partition_point(|line| line.sequence < sequence);
            let entry = rpc::TableLine {
                sequence,
                actor_id: actor_id.to_owned(),
                speaker: speaker.chars().take(48).collect(),
                role: role as i32,
                hero_index,
                kind: kind as i32,
                text,
                roll,
            };
            let tapped = self.sink.get().map(|_| entry.clone());
            ring.lines.insert(position, entry);
            while ring.lines.len() > RING_LINES {
                ring.lines.pop_front();
            }
            // The hero's line replaces the hero's loader in the same view.
            if role == rpc::TableRole::Hero && kind == rpc::TableKind::Speech {
                ring.composing.retain(|mark| mark.hero_index != hero_index);
            }
            ring.version += 1;
            (ring.version, sequence, tapped)
        };
        self.wake.send_replace(version);
        if let (Some(sink), Some(line)) = (self.sink.get(), tapped) {
            sink.line(&line);
        }
        Some(sequence)
    }

    /// Marks `speaker` as writing a reply until the returned guard is dropped, so every screen
    /// can show that someone is thinking. Presentation only: nothing waits on it.
    pub(super) fn reply_pending(self: &Arc<Self>, speaker: &str) -> ReplyPending {
        if self.enabled {
            let version = {
                let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
                ring.replying.push(speaker.chars().take(48).collect());
                ring.version += 1;
                ring.version
            };
            self.wake.send_replace(version);
        }
        ReplyPending {
            log: self.clone(),
            speaker: speaker.chars().take(48).collect(),
        }
    }

    fn reply_done(&self, speaker: &str) {
        if !self.enabled {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(position) = ring.replying.iter().position(|known| known == speaker) {
                ring.replying.remove(position);
            }
            ring.version += 1;
            ring.version
        };
        self.wake.send_replace(version);
    }

    /// Records what hero `hero_index` is doing in the dialogue box. Every refresh extends the
    /// mark's life and republishes the view, so a display that stops hearing it expires it too.
    /// Presentation only: nothing is committed and no game revision moves.
    pub(super) fn set_composing(self: &Arc<Self>, hero_index: u32, kind: rpc::ComposeKind) {
        let now = Instant::now();
        if let Some(deadline) = self.set_composing_at(hero_index, kind, now) {
            self.arm_reaper(deadline);
        }
    }

    /// The deadline of the earliest mark, when one is waiting to expire.
    fn set_composing_at(
        &self,
        hero_index: u32,
        kind: rpc::ComposeKind,
        now: Instant,
    ) -> Option<Instant> {
        if !self.enabled || hero_index >= PRESENCE_HEROES {
            return None;
        }
        let (changed, version, earliest) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let before = ring
                .composing
                .iter()
                .find(|mark| mark.hero_index == hero_index)
                .map(|mark| mark.kind);
            ring.composing.retain(|mark| mark.hero_index != hero_index);
            let shown = matches!(
                kind,
                rpc::ComposeKind::Typing | rpc::ComposeKind::Speaking | rpc::ComposeKind::Submitted
            );
            if shown {
                ring.composing.push(Composing {
                    hero_index,
                    kind,
                    deadline: now + self.expiry,
                });
            }
            // A refresh republishes too: it is the heartbeat the display expires the mark by.
            let changed = shown || before.is_some();
            if changed {
                ring.presence_version += 1;
            }
            let earliest = ring.composing.iter().map(|mark| mark.deadline).min();
            (changed, ring.version + ring.presence_version, earliest)
        };
        if changed {
            self.wake.send_replace(version);
        }
        earliest
    }

    /// Drops the marks whose time is up; republishes if any went. Returns the next deadline.
    fn expire_at(&self, now: Instant) -> Option<Instant> {
        let (removed, version, next) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let before = ring.composing.len();
            ring.composing.retain(|mark| mark.deadline > now);
            let removed = ring.composing.len() != before;
            if removed {
                ring.presence_version += 1;
            }
            let next = ring.composing.iter().map(|mark| mark.deadline).min();
            if next.is_none() {
                ring.reaper_armed = false;
            }
            (removed, ring.version + ring.presence_version, next)
        };
        if removed {
            self.wake.send_replace(version);
        }
        next
    }

    /// One task at a time waits for the earliest mark to expire, then for the next, so a closed
    /// tab clears from every screen without any other event.
    fn arm_reaper(self: &Arc<Self>, deadline: Instant) {
        let Some(runtime) = self.runtime() else {
            return;
        };
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if ring.reaper_armed {
                return;
            }
            ring.reaper_armed = true;
        }
        let log = self.clone();
        runtime.spawn(async move {
            let mut deadline = deadline;
            loop {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                match log.expire_at(Instant::now()) {
                    Some(next) => deadline = next,
                    None => break,
                }
            }
        });
    }

    /// Feeds one projected combat view to the pacing gate and returns `(released, arm_at)`:
    /// the sequence phones may act past right now, and the instant to wake watchers at when
    /// a release is pending. `durations` pairs each outcome's sequence with its nominal
    /// presentation time. A latest sequence below the released one means a new fight
    /// (sequences restart), so the gate resets instead of comparing across fights.
    fn pacing_observe(
        &self,
        latest: u64,
        durations: &[(u64, u64)],
        now: Instant,
    ) -> (u64, Option<Instant>) {
        let window = |from: u64, to: u64| -> u64 {
            durations
                .iter()
                .filter(|(seq, _)| *seq > from && *seq <= to)
                .map(|(_, ms)| *ms)
                .sum()
        };
        let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        if latest < ring.pacing.latest_known() {
            ring.pacing = Pacing::CaughtUp { released: 0 };
            ring.acks.clear();
        }
        // A due release (and any report that aged out) lands first, so beats committed
        // during the wait start a fresh wait of their own instead of extending one
        // already served. No wake is needed: the caller is publishing this view.
        let _advanced = Self::advance_pacing_locked(&mut ring, now);
        let mut pacing = ring.pacing;
        pacing = match pacing {
            Pacing::CaughtUp { released } if latest <= released => {
                Pacing::CaughtUp { released: latest }
            }
            Pacing::CaughtUp { released } => {
                let wait = window(released, latest).min(PACING_MAX_WAIT_MS);
                if wait == 0 {
                    Pacing::CaughtUp { released: latest }
                } else {
                    Pacing::Presenting {
                        released,
                        latest,
                        stamped_at: now,
                        release_at: now + Duration::from_millis(wait),
                    }
                }
            }
            Pacing::Presenting {
                released,
                latest: known,
                stamped_at,
                release_at,
            } => {
                if latest <= known {
                    Pacing::Presenting {
                        released,
                        latest: known,
                        stamped_at,
                        release_at,
                    }
                } else {
                    let cap = stamped_at + Duration::from_millis(PACING_MAX_WAIT_MS);
                    let extended = release_at + Duration::from_millis(window(known, latest));
                    Pacing::Presenting {
                        released,
                        latest,
                        stamped_at,
                        release_at: extended.min(cap),
                    }
                }
            }
        };
        ring.pacing = pacing;
        let arm = Self::next_wake_locked(&ring);
        (pacing.released(), arm)
    }

    /// The pacing timer's wake: lands a due release, drops displays that stopped
    /// reporting (releasing what the remaining fresh reports allow), and republishes
    /// so the phones are offered the next action without waiting for another event.
    /// Returns the next wake instant when a wait remains; the armed marker is cleared
    /// in the same lock as the release, by the timer task itself, so it can never be
    /// left set for a wait no task is watching.
    fn expire_pacing_at(&self, now: Instant) -> Option<Instant> {
        let (advanced, version, next) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let story_released = Self::release_story_locked(&mut ring, now);
            let advanced = Self::advance_pacing_locked(&mut ring, now) || story_released;
            if advanced {
                ring.version += 1;
            }
            let next = Self::next_wake_locked(&ring);
            ring.pacing_armed_until = next;
            (advanced, ring.version + ring.presence_version, next)
        };
        if advanced {
            self.wake.send_replace(version);
        }
        next
    }

    /// The sequence the pacing gate has released so far.
    pub(super) fn released_sequence(&self) -> u64 {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pacing
            .released()
    }

    /// Display instance `display` finished presenting beat `sequence`. Only the
    /// newest report per instance is kept, clamped to the beats actually pending
    /// (never beyond latest); the slowest fresh instance governs, and a report can do
    /// nothing but advance the released sequence — a reconnected display reporting
    /// from the start therefore never moves it backward. Also arms the pacing timer
    /// for the new wake (this report's own ageing-out), whichever thread it runs on.
    /// Returns the released sequence after the report.
    pub(super) fn report_presented_at(
        self: &Arc<Self>,
        display: u64,
        sequence: u64,
        now: Instant,
    ) -> u64 {
        if !self.enabled {
            return sequence;
        }
        let (released, advanced, version, wake_at) = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let Pacing::Presenting { latest, .. } = ring.pacing else {
                return ring.pacing.released();
            };
            if !ring.acks.contains_key(&display) && ring.acks.len() >= ACK_DISPLAYS {
                // Bounded: the stalest tracked display makes room for a new one.
                if let Some(stalest) = ring
                    .acks
                    .iter()
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(instance, _)| *instance)
                {
                    ring.acks.remove(&stalest);
                }
            }
            ring.acks.insert(display, (sequence.min(latest), now));
            let advanced = Self::advance_pacing_locked(&mut ring, now);
            if advanced {
                ring.version += 1;
            }
            (
                ring.pacing.released(),
                advanced,
                ring.version + ring.presence_version,
                Self::next_wake_locked(&ring),
            )
        };
        if advanced {
            self.wake.send_replace(version);
        }
        if let Some(at) = wake_at {
            self.arm_pacing(at);
        }
        released
    }

    /// Applies the gate to one view about to be published: stamps the released sequence
    /// on the combat view, withholds a player's offers while beats are unreleased, and
    /// arms the release timer. The display is never gated; it presents the beats.
    fn apply_pacing(self: &Arc<Self>, view: &mut rpc::ViewMessage) {
        let combat = match view.audience.as_ref() {
            Some(rpc::view_message::Audience::Player(player)) => player
                .journey
                .as_ref()
                .and_then(|journey| journey.combat.as_ref()),
            Some(rpc::view_message::Audience::Display(display)) => display
                .journey
                .as_ref()
                .and_then(|journey| journey.combat.as_ref()),
            None => None,
        };
        let Some(combat) = combat else {
            self.apply_story_hold(view, Instant::now());
            return;
        };
        let latest = combat.latest_sequence;
        let durations: Vec<(u64, u64)> = combat
            .outcomes
            .iter()
            .chain(&combat.move_outcomes)
            .chain(&combat.death_save_outcomes)
            .filter(|outcome| outcome.sequence > 0)
            .map(|outcome| (outcome.sequence, beat_duration_ms(outcome)))
            .collect();
        let (released, arm) = self.pacing_observe(latest, &durations, Instant::now());
        let presenting = released < latest;
        match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Player(player)) => {
                if presenting {
                    player.offers.clear();
                    player.action_available = false;
                }
                if let Some(combat) = player
                    .journey
                    .as_mut()
                    .and_then(|journey| journey.combat.as_mut())
                {
                    combat.released_sequence = released;
                }
            }
            Some(rpc::view_message::Audience::Display(display)) => {
                if let Some(combat) = display
                    .journey
                    .as_mut()
                    .and_then(|journey| journey.combat.as_mut())
                {
                    combat.released_sequence = released;
                }
            }
            None => {}
        }
        if let Some(at) = arm {
            self.arm_pacing(at);
        }
    }

    /// Outside a fight: a player's view of a story scene the phones have not been shown before
    /// starts the hold, and while it lasts such a view carries no choices; the timer that ends
    /// the hold is armed. The display is never gated, and the lobby and hero creation are
    /// untouched (their offers are not story choices).
    fn apply_story_hold(self: &Arc<Self>, view: &mut rpc::ViewMessage, now: Instant) {
        let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_mut() else {
            return;
        };
        let Some(journey) = player.journey.as_ref() else {
            return;
        };
        if journey.creation.is_some()
            || !matches!(
                rpc::JourneyPhase::try_from(journey.phase),
                Ok(rpc::JourneyPhase::Opening
                    | rpc::JourneyPhase::Dialogue
                    | rpc::JourneyPhase::Complete)
            )
        {
            return;
        }
        let scene = player.scene.as_ref();
        let shown = (
            journey.phase,
            scene.map_or_else(String::new, |scene| scene.title.clone()),
            scene.map_or_else(String::new, |scene| scene.place.clone()),
        );
        let until = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let _released = Self::release_story_locked(&mut ring, now);
            if !ring.story_scenes.contains(&shown) {
                if ring.story_scenes.len() == 2 {
                    ring.story_scenes.remove(0);
                }
                ring.story_scenes.push(shown);
                ring.story_hold_until = Some(now + Duration::from_millis(STORY_HOLD_MS));
                println!("STORY hold ms={STORY_HOLD_MS} reason=telling");
            }
            ring.story_hold_until
        };
        let Some(until) = until else {
            return;
        };
        player.offers.clear();
        player.action_available = false;
        self.arm_pacing(until);
    }

    /// Arms the pacing timer whatever thread asks: the runtime handle captured at
    /// construction or by a watcher subscription is used when the caller (the
    /// session owner thread) has none of its own. A running task follows the gate's
    /// next wake by itself, so a second task is spawned only when no task is running
    /// or the new wake is earlier than the one it sleeps to. When no handle exists
    /// yet the failure is logged once per wait and the marker stays unset, so the
    /// next subscription, view or report retries the arm instead of stalling.
    fn arm_pacing(self: &Arc<Self>, deadline: Instant) {
        let Some(runtime) = self.runtime() else {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if !ring.pacing_arm_failed {
                ring.pacing_arm_failed = true;
                println!("PACING timer_unarmed step=arm_pacing reason=no_runtime_handle");
            }
            return;
        };
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            ring.pacing_arm_failed = false;
            if ring.pacing_armed_until.is_some_and(|at| at <= deadline) {
                return;
            }
            ring.pacing_armed_until = Some(deadline);
        }
        let log = self.clone();
        runtime.spawn(async move {
            let mut deadline = deadline;
            loop {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                match log.expire_pacing_at(Instant::now()) {
                    Some(next) => deadline = next,
                    None => break,
                }
            }
        });
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> Vec<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .cloned()
            .collect()
    }

    /// The newest public lines, oldest first, as (speaker, text): the table's recent talk, for a
    /// generator that must agree with what was just said. Roll rows are left out.
    pub(super) fn recent_public(&self, count: usize) -> Vec<(String, String)> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let mut lines: Vec<(String, String)> = ring
            .lines
            .iter()
            .rev()
            .filter(|line| line.speaker != "Roll" && !line.text.trim().is_empty())
            .take(count)
            .map(|line| (line.speaker.clone(), line.text.clone()))
            .collect();
        lines.reverse();
        lines
    }

    /// Checks the exact current NPC speech line that a Jev proposal names. Sequence is
    /// captured when the reply enters the table log, so another character's later line
    /// cannot make a previous NPC proposal look current by matching its text.
    pub(super) fn is_latest_npc_speech(
        &self,
        sequence: u64,
        actor_id: &str,
        speaker: &str,
        text: &str,
    ) -> bool {
        self.latest_npc_speech_text(sequence, actor_id, speaker, text)
            .is_some()
    }

    /// Returns the latest in-memory NPC speech when its actor and sequence match. This is for
    /// checks on live table turns; durable quote receipts are validated from the checkpoint
    /// journal and must not depend on this bounded presentation ring.
    pub(super) fn latest_npc_speech_at(&self, sequence: u64, actor_id: &str) -> Option<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        ring.lines
            .iter()
            .rev()
            .find(|line| {
                line.role == rpc::TableRole::Npc as i32
                    && line.kind == rpc::TableKind::Speech as i32
            })
            .filter(|line| line.sequence == sequence && line.actor_id == actor_id)
            .map(|line| line.text.clone())
    }

    /// Returns one public line by its stable in-memory sequence. Transaction callers use this to
    /// bind a durable decision to exactly what the table displayed, never to raw model or input
    /// text that may have been cleaned or redacted.
    pub(super) fn line_at_sequence(&self, sequence: u64) -> Option<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .find(|line| line.sequence == sequence)
            .cloned()
    }

    pub(super) fn latest_hero_speech_after(&self, sequence: u64) -> Option<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .rev()
            .find(|line| {
                line.sequence > sequence
                    && line.role == rpc::TableRole::Hero as i32
                    && line.kind == rpc::TableKind::Speech as i32
            })
            .cloned()
    }

    pub(super) fn latest_hero_speech_before(&self, sequence: u64) -> Option<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .rev()
            .find(|line| {
                line.sequence < sequence
                    && line.role == rpc::TableRole::Hero as i32
                    && line.kind == rpc::TableKind::Speech as i32
            })
            .cloned()
    }

    pub(super) fn latest_npc_speech_before(&self, sequence: u64) -> Option<rpc::TableLine> {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .iter()
            .rev()
            .find(|line| {
                line.sequence < sequence
                    && line.role == rpc::TableRole::Npc as i32
                    && line.kind == rpc::TableKind::Speech as i32
            })
            .cloned()
    }

    /// Returns the exact text the table displayed for the current NPC source line. Callers
    /// that journal a source-bound decision persist this value instead of model output that
    /// may contain control characters or exceed the public line limit.
    pub(super) fn latest_npc_speech_text(
        &self,
        sequence: u64,
        actor_id: &str,
        speaker: &str,
        text: &str,
    ) -> Option<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        ring.lines
            .iter()
            .rev()
            .find(|line| {
                line.role == rpc::TableRole::Npc as i32
                    && line.kind == rpc::TableKind::Speech as i32
            })
            .filter(|line| {
                line.sequence == sequence
                    && line.actor_id == actor_id
                    && line.speaker == speaker.chars().take(48).collect::<String>()
                    && line.text == visible_table_line(text)
            })
            .map(|line| line.text.clone())
    }

    /// Adds the public feed to the view of either audience. In a fight it also runs the
    /// pacing gate: the view's combat releases only the beats whose presentation wait
    /// has elapsed, and a phone's offers are withheld until then (the phone reads
    /// `released_sequence < latest_sequence` as "resolving on the TV").
    pub(super) fn attach(self: &Arc<Self>, view: &mut rpc::ViewMessage) {
        if !self.enabled {
            return;
        }
        self.apply_pacing(view);
        let display = matches!(view.audience, Some(rpc::view_message::Audience::Display(_)));
        let journey = match view.audience.as_mut() {
            Some(rpc::view_message::Audience::Player(player)) => player.journey.as_mut(),
            Some(rpc::view_message::Audience::Display(display)) => display.journey.as_mut(),
            None => None,
        };
        if let Some(journey) = journey {
            let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            journey.table_log = ring.lines.iter().cloned().collect();
            journey.presentation_sequence = ring.version;
            journey.reply_speaker = ring.replying.first().cloned().unwrap_or_default();
            if !display {
                return;
            }
            journey.presentation_sequence += ring.presence_version;
            let now = Instant::now();
            journey.composing = ring
                .composing
                .iter()
                .filter(|mark| mark.deadline > now)
                .map(|mark| rpc::ComposingHero {
                    hero_index: mark.hero_index,
                    kind: mark.kind as i32,
                })
                .collect();
        }
    }
}

/// Keeps a speaker marked as replying; dropping it (reply written, failed or abandoned) clears
/// the mark.
pub(super) struct ReplyPending {
    log: Arc<TableLog>,
    speaker: String,
}

impl Drop for ReplyPending {
    fn drop(&mut self) {
        self.log.reply_done(&self.speaker);
    }
}

/// One public line an accepted action produces.
pub(super) struct ActionLine {
    pub(super) speaker: String,
    pub(super) text: String,
    /// The d20 roll the line tells, when it tells one the display plays (a death save, a
    /// stabilise check). Attacks play from their combat outcome and carry none here.
    pub(super) roll: Option<rpc::TableRoll>,
}

/// The public lines of an accepted action: the acting hero's short phrase from the offer's
/// label and visible result, then what the enemy did in answer. Some actions have no story phrase.
pub(super) fn action_lines(
    kind: rpc::GameplayActionKind,
    hero: &str,
    combat: &[rpc::CombatOutcome],
    weapon: &str,
    destination: &str,
    label: &str,
) -> Vec<ActionLine> {
    use rpc::GameplayActionKind as Kind;
    let own = combat
        .iter()
        .find(|outcome| outcome.actor_name == hero && is_attack(outcome));
    let stabilisation = combat.iter().find(|outcome| {
        outcome.actor_name == hero
            && rpc::OutcomeKind::try_from(outcome.kind) == Ok(rpc::OutcomeKind::Stabilisation)
    });
    // The resolved attack names the weapon it used (a longbow for an archer); the hero's melee
    // weapon is only the fallback when the outcome carries none.
    let used = own
        .filter(|outcome| !outcome.weapon.is_empty())
        .map_or_else(
            || weapon.to_owned(),
            |outcome| outcome.weapon.replace('-', " "),
        );
    if matches!(kind, Kind::CreateCharacter | Kind::Unspecified) {
        return Vec::new();
    }
    // Only an attack action is worded from the attack it resolved; moves keep their own line even
    // when the same submit carries a reaction's blow. Ending a turn has no story phrase.
    let attacking = matches!(kind, Kind::GreatswordAttack | Kind::OpportunityAttack);
    let phrase = if let Some(outcome) = own.filter(|_| attacking) {
        // An unarmed strike is the attack itself, not a thing held: "makes an unarmed strike",
        // never "strikes with the unarmed strike".
        let unarmed = used == "unarmed strike";
        let attack = match attack_action_verb(outcome) {
            AttackActionVerb::Throw => format!("throws the {used}"),
            AttackActionVerb::Shoot => format!("shoots the {used}"),
            AttackActionVerb::Strike if outcome.grazed || !outcome.hit => {
                if unarmed {
                    "swings an unarmed strike".to_owned()
                } else {
                    format!("swings the {used}")
                }
            }
            AttackActionVerb::Strike if unarmed => "makes an unarmed strike".to_owned(),
            AttackActionVerb::Strike => format!("strikes with the {used}"),
        };
        let result = if outcome.grazed {
            format!("graze, {} damage", damage_words(outcome))
        } else if outcome.hit {
            format!(
                "{}hit, {} damage",
                if outcome.critical { "critical " } else { "" },
                damage_words(outcome)
            )
        } else {
            "miss".to_owned()
        };
        if kind == Kind::OpportunityAttack {
            format!(
                "makes an opportunity attack on {}: {attack} \u{2014} {result}",
                outcome.target_name
            )
        } else {
            format!("{attack} \u{2014} {result}")
        }
    } else if let Some(outcome) = stabilisation {
        match &outcome.stabilization {
            Some(result) if result.succeeded => format!(
                "steadies {}: {} is stable",
                outcome.target_name, outcome.target_name
            ),
            Some(_) => format!(
                "tries to steady {}, but the Medicine check fails",
                outcome.target_name
            ),
            None => format!("acts to steady {}", outcome.target_name),
        }
    } else {
        match kind {
            Kind::GreatswordAttack if used == "unarmed strike" => {
                "makes an unarmed strike".to_owned()
            }
            Kind::GreatswordAttack => format!("strikes with the {used}"),
            Kind::SecondWind => "catches a second wind".to_owned(),
            Kind::ShortRest => "takes a short rest".to_owned(),
            Kind::Move => {
                let (squares, feet) = combat
                    .iter()
                    .filter(|outcome| {
                        outcome.actor_name == hero
                            && rpc::OutcomeKind::try_from(outcome.kind)
                                == Ok(rpc::OutcomeKind::Move)
                    })
                    .fold((0, 0), |(squares, feet), outcome| {
                        (squares + outcome.path.len(), feet + outcome.cost_ft)
                    });
                format!(
                    "moves {squares} square{} ({feet} ft)",
                    if squares == 1 { "" } else { "s" }
                )
            }
            Kind::ChooseHarborScene => format!("leads on to {destination}"),
            Kind::ResolveErrand => third_person_or_choice(label),
            _ => third_person(label),
        }
    };
    // A stabilise check is the hero's own roll: it carries the Medicine die for the display.
    let phrase_roll = stabilisation
        .filter(|_| !(attacking && own.is_some()))
        .and_then(crate::table_roll::outcome_roll);
    // Starting a fight has no hero phrase of its own: the offer's label ("Start a fight") is a
    // button caption, not the hero's words. The hero's typed line (published by the dialogue
    // route) or the narrator's fight-start lines tell the act; only the combat rows stand here.
    let mut phrase_line = if matches!(kind, Kind::BeginStory | Kind::EndTurn | Kind::SceneAssault) {
        None
    } else {
        Some(ActionLine {
            speaker: hero.to_owned(),
            text: phrase,
            roll: phrase_roll,
        })
    };
    // Lines follow the server's resolution order: the hero's own line stands where the hero's
    // first outcome stands, so a reaction that came before the hero's move is told before it.
    let anchor = combat
        .iter()
        .position(|outcome| outcome.actor_name == hero)
        .unwrap_or(0);
    let mut lines = Vec::new();
    let mut told_saves: Vec<(String, u32, u32, u32, i32)> = Vec::new();
    let mut index = 0;
    while index < combat.len() {
        if index == anchor {
            lines.extend(phrase_line.take());
        }
        let outcome = &combat[index];
        index += 1;
        match rpc::OutcomeKind::try_from(outcome.kind) {
            Ok(rpc::OutcomeKind::DeathSave) => {
                // One Death Saving Throw is told once, however many times it is handed over.
                let key = (
                    outcome.actor_name.clone(),
                    outcome.attack_die,
                    outcome.death_successes,
                    outcome.death_failures,
                    outcome.death_save_result,
                );
                if told_saves.contains(&key) {
                    continue;
                }
                told_saves.push(key);
                lines.push(ActionLine {
                    speaker: outcome.actor_name.clone(),
                    text: death_save_text(outcome),
                    roll: crate::table_roll::outcome_roll(outcome),
                });
                continue;
            }
            Ok(rpc::OutcomeKind::Recovery) => {
                let unit = if outcome.recovery_hours == 1 {
                    "hour"
                } else {
                    "hours"
                };
                lines.push(ActionLine {
                    speaker: "Narrator".to_owned(),
                    text: format!(
                        "{} has been stable for {} {unit} and regains 1 hit point",
                        outcome.actor_name, outcome.recovery_hours
                    ),
                    roll: None,
                });
                continue;
            }
            Ok(rpc::OutcomeKind::Move) => {
                if outcome.actor_name != hero {
                    lines.push(ActionLine {
                        speaker: outcome.actor_name.clone(),
                        text: format!(
                            "moves {} square{} ({} ft)",
                            outcome.path.len(),
                            if outcome.path.len() == 1 { "" } else { "s" },
                            outcome.cost_ft
                        ),
                        roll: None,
                    });
                }
                continue;
            }
            Ok(rpc::OutcomeKind::Stabilisation) => continue,
            _ => {}
        }
        if outcome.actor_name == hero {
            continue;
        }
        // A Multiattack is one line: the same attacker's consecutive attacks on one target.
        let mut group = vec![outcome];
        while let Some(next) = combat.get(index) {
            if next.kind == outcome.kind
                && next.kind == rpc::OutcomeKind::Attack as i32
                && next.actor_name == outcome.actor_name
                && next.target_name == outcome.target_name
            {
                group.push(next);
                index += 1;
            } else {
                break;
            }
        }
        lines.push(ActionLine {
            speaker: outcome.actor_name.clone(),
            text: enemy_attack_text(&group),
            roll: None,
        });
    }
    lines.extend(phrase_line);
    lines
}

/// The word for how many attacks a Multiattack made.
fn attack_count_word(count: usize) -> String {
    match count {
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        count => count.to_string(),
    }
}

/// Whether a blow dropped its target: knocked out (unconscious at 1 HP) or down at 0 HP.
fn blow_dropped_target(outcome: &rpc::CombatOutcome) -> Option<&'static str> {
    if outcome.knocked_out {
        Some("falls unconscious")
    } else if outcome.hit && outcome.target_hit_points == 0 {
        Some("is down")
    } else {
        None
    }
}

/// The public sentence for what an opponent did with one attack, an opportunity attack or a
/// Multiattack (several consecutive attacks of one attacker on one target, in resolution order).
fn enemy_attack_text(group: &[&rpc::CombatOutcome]) -> String {
    let Some(first) = group.first() else {
        return String::new();
    };
    let result_of = |outcome: &rpc::CombatOutcome| {
        if outcome.hit {
            format!(
                "{}hit, {} damage",
                if outcome.critical { "critical " } else { "" },
                damage_words(outcome)
            )
        } else if outcome.grazed {
            format!("graze, {} damage", damage_words(outcome))
        } else {
            "miss".to_owned()
        }
    };
    let down = group
        .iter()
        .find_map(|outcome| blow_dropped_target(outcome))
        .map(|words| format!(", {} {words}", first.target_name))
        .unwrap_or_default();
    if group.len() > 1 {
        let results = group
            .iter()
            .map(|outcome| result_of(outcome))
            .collect::<Vec<_>>()
            .join("; ");
        return format!(
            "makes {} attacks on {} \u{2014} {results}{down}",
            attack_count_word(group.len()),
            first.target_name
        );
    }
    let outcome = first;
    if rpc::OutcomeKind::try_from(outcome.kind) == Ok(rpc::OutcomeKind::OpportunityAttack) {
        return format!(
            "makes an opportunity attack on {} \u{2014} {}{down}",
            outcome.target_name,
            result_of(outcome)
        );
    }
    if outcome.hit {
        let weapon = outcome.weapon.replace('-', " ");
        let attack = match attack_action_verb(outcome) {
            AttackActionVerb::Throw if !weapon.is_empty() => {
                format!("throws the {weapon} at {}", outcome.target_name)
            }
            AttackActionVerb::Throw => format!("throws at {}", outcome.target_name),
            AttackActionVerb::Shoot if !weapon.is_empty() => {
                format!("shoots at {} with the {weapon}", outcome.target_name)
            }
            AttackActionVerb::Shoot => format!("shoots at {}", outcome.target_name),
            AttackActionVerb::Strike => format!("strikes {}", outcome.target_name),
        };
        format!(
            "{attack} \u{2014} {}hit, {} damage{down}",
            if outcome.critical { "critical " } else { "" },
            damage_words(outcome)
        )
    } else {
        format!("misses {}", outcome.target_name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttackActionVerb {
    Throw,
    Shoot,
    Strike,
}

fn attack_action_verb(outcome: &rpc::CombatOutcome) -> AttackActionVerb {
    if outcome.attack_style != 2 {
        return AttackActionVerb::Strike;
    }
    // SRD 5.2.1 pp. 90-91: the Thrown property makes a ranged attack by throwing the weapon.
    if matches!(
        outcome.weapon.as_str(),
        "club"
            | "dagger"
            | "handaxe"
            | "javelin"
            | "light-hammer"
            | "spear"
            | "trident"
            | "dart"
            | "net"
    ) {
        AttackActionVerb::Throw
    } else {
        AttackActionVerb::Shoot
    }
}

/// The damage dealt with its type when the rules named one: "9 slashing".
fn damage_words(outcome: &rpc::CombatOutcome) -> String {
    if outcome.damage_type.is_empty() {
        outcome.damage.to_string()
    } else {
        format!("{} {}", outcome.damage, outcome.damage_type)
    }
}

/// One server log line per recorded outcome that proves the turn was legal: the squares and
/// feet a move spent, the distance and mode of an attack with what changed its dice, and each
/// Death Saving Throw with the running counts.
pub(super) fn audit_line(outcome: &rpc::CombatOutcome) -> String {
    let squares = |cells: &[rpc::GridCell]| {
        cells
            .iter()
            .map(|cell| format!("({},{})", cell.col, cell.row))
            .collect::<Vec<_>>()
            .join("")
    };
    match rpc::OutcomeKind::try_from(outcome.kind) {
        Ok(rpc::OutcomeKind::Move) => format!(
            "COMBAT move actor={:?} squares={} path={} cost_ft={} movement_left_ft={}",
            outcome.actor_name,
            outcome.path.len(),
            squares(&outcome.path),
            outcome.cost_ft,
            outcome.remaining_ft
        ),
        Ok(rpc::OutcomeKind::Recovery) => format!(
            "COMBAT stable_recovery actor={:?} hours={}",
            outcome.actor_name, outcome.recovery_hours
        ),
        Ok(rpc::OutcomeKind::Stabilisation) => match &outcome.stabilization {
            Some(result) => match rpc::StabilizationMethod::try_from(result.method) {
                Ok(rpc::StabilizationMethod::HealersKit) => format!(
                    "COMBAT stabilise actor={:?} target={:?} method=kit uses_left={}",
                    outcome.actor_name, outcome.target_name, result.uses_left
                ),
                Ok(rpc::StabilizationMethod::Medicine) => format!(
                    "COMBAT stabilise actor={:?} target={:?} method=medicine die={} modifier={} dc={} result={}",
                    outcome.actor_name,
                    outcome.target_name,
                    result.die,
                    result.wisdom_modifier,
                    result.dc,
                    if result.succeeded {
                        "success"
                    } else {
                        "failure"
                    }
                ),
                _ => format!(
                    "COMBAT stabilise actor={:?} target={:?} method=unspecified",
                    outcome.actor_name, outcome.target_name
                ),
            },
            None => format!(
                "COMBAT stabilise actor={:?} target={:?} method=unspecified",
                outcome.actor_name, outcome.target_name
            ),
        },
        Ok(rpc::OutcomeKind::DeathSave) => format!(
            "COMBAT death_save actor={:?} die={} result={:?} successes={} failures={}",
            outcome.actor_name,
            outcome.attack_die,
            rpc::DeathSaveResult::try_from(outcome.death_save_result)
                .unwrap_or(rpc::DeathSaveResult::Unspecified),
            outcome.death_successes,
            outcome.death_failures
        ),
        kind => format!(
            "COMBAT attack actor={:?} target={:?} weapon={} mode={}{} distance_ft={} cover={} roll_mode={} faces={:?} modifier={} hit={} critical={} damage={}{} reasons={:?}",
            outcome.actor_name,
            outcome.target_name,
            outcome.weapon,
            if outcome.attack_style == 2 {
                "ranged"
            } else {
                "melee"
            },
            if kind == Ok(rpc::OutcomeKind::OpportunityAttack) {
                " reaction=opportunity"
            } else {
                ""
            },
            outcome.distance_ft,
            outcome.cover,
            outcome.roll_mode,
            outcome.faces,
            outcome.attack_modifier,
            outcome.hit,
            outcome.critical,
            outcome.damage,
            // The dice behind the damage, when the fight engine recorded them.
            if outcome.damage_dice.is_empty() {
                String::new()
            } else {
                format!(" damage_faces={:?}", outcome.damage_dice)
            },
            outcome.reasons
        ),
    }
}

pub(super) fn initiative_audit_line(rolls: &[rpc::InitiativeRoll]) -> String {
    let order = rolls
        .iter()
        .map(|roll| format!("{}:{}", roll.name, roll.total))
        .collect::<Vec<_>>()
        .join(", ");
    format!("COMBAT initiative order=[{order}]")
}

pub(super) fn combat_audit_lines(
    outcomes: &[rpc::CombatOutcome],
    before: Option<&rpc::CombatView>,
    after: Option<&rpc::CombatView>,
) -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let mut actor = before
        .map_or("", |combat| combat.active_actor.as_str())
        .to_owned();
    let mut round = before.map_or(1, |combat| combat.round);
    let turn_order = after
        .into_iter()
        .flat_map(|combat| &combat.initiative)
        .map(|roll| roll.name.as_str())
        .collect::<Vec<_>>();
    for outcome in outcomes {
        let opportunity_attack =
            rpc::OutcomeKind::try_from(outcome.kind) == Ok(rpc::OutcomeKind::OpportunityAttack);
        if !opportunity_attack && outcome.actor_name != actor {
            advance_audit_round(&actor, &outcome.actor_name, &turn_order, &mut round);
            lines.push((
                "gameplay.combat_turn",
                turn_audit_line(&outcome.actor_name, round),
            ));
            actor.clone_from(&outcome.actor_name);
        }
        lines.push(("gameplay.combat_outcome", audit_line(outcome)));
    }
    if let Some(after) = after
        && !after.active_actor.is_empty()
        && after.active_actor != actor
    {
        advance_audit_round(&actor, &after.active_actor, &turn_order, &mut round);
        lines.push((
            "gameplay.combat_turn",
            turn_audit_line(&after.active_actor, round),
        ));
    }
    lines
}

fn advance_audit_round(actor: &str, next: &str, order: &[&str], round: &mut u32) {
    let current = order.iter().position(|name| *name == actor);
    let next = order.iter().position(|name| *name == next);
    if current
        .zip(next)
        .is_some_and(|(current, next)| next <= current)
    {
        *round = round.saturating_add(1);
    }
}

pub(super) fn turn_audit_line(actor: &str, round: u32) -> String {
    format!("COMBAT turn actor={actor:?} round={round}")
}

pub(super) fn second_wind_audit_line(
    actor: &str,
    healed: u32,
    hit_points: u32,
    uses_left: u32,
) -> String {
    format!(
        "COMBAT second-wind actor={actor:?} healed={healed} hit_points={hit_points} uses_left={uses_left}"
    )
}

pub(super) fn end_turn_audit_line(actor: &str) -> String {
    format!("COMBAT end-turn actor={actor:?}")
}

pub(super) fn combat_action_audit_line(action: &str, actor: &str) -> String {
    format!("COMBAT {action} actor={actor:?}")
}

pub(super) fn fight_end_audit_line(outcome: &str, rounds: u32) -> String {
    format!("COMBAT fight-end outcome={outcome} rounds={rounds}")
}

pub(super) fn story_choice_audit_line(
    actor: &str,
    action: &str,
    label: &str,
    phase_before: &str,
    phase_after: &str,
) -> String {
    format!(
        "STORY choice actor={actor:?} action={action} label={label:?} phase={phase_before}->{phase_after}"
    )
}

pub(super) fn story_rest_audit_line(actor: &str) -> String {
    format!("STORY rest actor={actor:?}")
}

pub(super) fn story_rest_hit_die_audit_line(
    actor: &str,
    hit_die: u32,
    modifier: i32,
    healed: u32,
    hit_points: u32,
) -> String {
    format!(
        "STORY rest actor={actor:?} hit_die={hit_die} modifier={modifier} healed={healed} hit_points={hit_points}"
    )
}

pub(super) fn story_say_audit_line(actor: &str, chars: usize, target: &str) -> String {
    format!("STORY say actor={actor:?} chars={chars} target={target:?}")
}

fn is_attack(outcome: &rpc::CombatOutcome) -> bool {
    !matches!(
        rpc::OutcomeKind::try_from(outcome.kind),
        Ok(rpc::OutcomeKind::Move | rpc::OutcomeKind::DeathSave | rpc::OutcomeKind::Recovery)
    )
}

/// The public sentence for one Death Saving Throw: the roll, the result, the running counts.
fn death_save_text(outcome: &rpc::CombatOutcome) -> String {
    let result = match rpc::DeathSaveResult::try_from(outcome.death_save_result) {
        Ok(rpc::DeathSaveResult::Success) => "success",
        Ok(rpc::DeathSaveResult::Failure) => "failure",
        Ok(rpc::DeathSaveResult::CriticalFailure) => "a natural 1, two failures",
        Ok(rpc::DeathSaveResult::Stabilised) => "stable at last",
        Ok(rpc::DeathSaveResult::Died) => "dies",
        Ok(rpc::DeathSaveResult::Revived) => "a natural 20, back on their feet with 1 hit point",
        _ => "no result",
    };
    let count = |number: u32, singular: &str, plural: &str| {
        format!("{number} {}", if number == 1 { singular } else { plural })
    };
    format!(
        "makes a Death Saving Throw: {} \u{2014} {result} ({}, {})",
        outcome.attack_die,
        count(outcome.death_successes, "success", "successes"),
        count(outcome.death_failures, "failure", "failures")
    )
}

/// The third-person phrase for a choice label, or its quoted form when the label has no action
/// verb at the start.
fn third_person_or_choice(label: &str) -> String {
    let first_word = label.split_whitespace().next().unwrap_or_default();
    let lower_first_word = first_word.to_lowercase();
    if first_word.is_empty()
        || matches!(lower_first_word.as_str(), "a" | "an" | "the")
        || first_word.contains('\'')
    {
        format!("chose \"{label}\"")
    } else {
        third_person(label)
    }
}

fn notebook_entry_log_fields(text: &str) -> Option<(&str, &str, &str)> {
    let text = text.strip_prefix("NOTE added kind=")?;
    let (kind, rest) = text.split_once(" title=")?;
    let (title, scene) = rest.rsplit_once(" scene=")?;
    let title = title.strip_prefix('"')?.strip_suffix('"')?;
    (!kind.is_empty() && !title.is_empty() && !scene.is_empty()).then_some((kind, title, scene))
}

/// The third-person phrase for an offer label: its imperative verb takes an `s`
/// ("Ask Ilsa what is wrong" becomes "asks Ilsa what is wrong").
pub(super) fn third_person(label: &str) -> String {
    let label = label.trim();
    let (verb, rest) = label.split_once(' ').unwrap_or((label, ""));
    let verb = verb.to_lowercase();
    let consonant_y = verb.len() > 1
        && verb.ends_with('y')
        && !verb[..verb.len() - 1].ends_with(['a', 'e', 'i', 'o', 'u']);
    let conjugated = if consonant_y {
        format!("{}ies", &verb[..verb.len() - 1])
    } else if verb.ends_with(['s', 'x', 'z', 'o']) || verb.ends_with("ch") || verb.ends_with("sh") {
        format!("{verb}es")
    } else {
        format!("{verb}s")
    };
    if rest.is_empty() {
        conjugated
    } else {
        format!("{conjugated} {}", third_person_words(rest))
    }
}

/// The second-person words of an imperative label, turned to the third person the log speaks in.
fn third_person_words(rest: &str) -> String {
    rest.split(' ')
        .map(|word| match word {
            "yourself" => "themselves",
            "your" => "their",
            "you" => "them",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initiative_audit_formatter_preserves_committed_order_and_totals() {
        let rolls = [
            rpc::InitiativeRoll {
                name: "Sigrun".to_owned(),
                total: 18,
                order: 1,
                ..Default::default()
            },
            rpc::InitiativeRoll {
                name: "Giant Weasel".to_owned(),
                total: 12,
                order: 2,
                ..Default::default()
            },
        ];
        assert_eq!(
            initiative_audit_line(&rolls),
            "COMBAT initiative order=[Sigrun:18, Giant Weasel:12]"
        );
    }

    #[test]
    fn turn_audit_formatter_quotes_the_actor_and_names_the_round() {
        assert_eq!(
            turn_audit_line("Brakka", 3),
            "COMBAT turn actor=\"Brakka\" round=3"
        );
    }

    #[test]
    fn second_wind_audit_formatter_records_the_heal_hp_and_uses() {
        assert_eq!(
            second_wind_audit_line("Brakka", 4, 12, 0),
            "COMBAT second-wind actor=\"Brakka\" healed=4 hit_points=12 uses_left=0"
        );
    }

    #[test]
    fn end_turn_audit_formatter_quotes_the_actor() {
        assert_eq!(
            end_turn_audit_line("Brakka"),
            "COMBAT end-turn actor=\"Brakka\""
        );
    }

    #[test]
    fn combat_action_audit_formatter_records_the_action_and_actor() {
        assert_eq!(
            combat_action_audit_line("dash", "Brakka"),
            "COMBAT dash actor=\"Brakka\""
        );
    }

    #[test]
    fn fight_end_audit_formatter_records_outcome_and_round_count() {
        assert_eq!(
            fight_end_audit_line("victory", 2),
            "COMBAT fight-end outcome=victory rounds=2"
        );
    }

    #[test]
    fn story_choice_audit_formatter_records_kind_label_and_phase_transition() {
        assert_eq!(
            story_choice_audit_line(
                "Brakka",
                "begin_story",
                "Begin the story",
                "character_creation",
                "opening"
            ),
            "STORY choice actor=\"Brakka\" action=begin_story label=\"Begin the story\" phase=character_creation->opening"
        );
    }

    #[test]
    fn combat_audit_orders_each_turn_before_that_actors_outcomes() {
        let outcome = |actor: &str| rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Attack as i32,
            actor_name: actor.to_owned(),
            ..Default::default()
        };
        let before = rpc::CombatView::default();
        let after = rpc::CombatView {
            active_actor: "Weasel".to_owned(),
            ..Default::default()
        };
        let outcomes = [outcome("Brakka"), outcome("Weasel")];
        let lines = combat_audit_lines(&outcomes, Some(&before), Some(&after));

        assert_eq!(
            lines.iter().map(|(event, _)| *event).collect::<Vec<_>>(),
            [
                "gameplay.combat_turn",
                "gameplay.combat_outcome",
                "gameplay.combat_turn",
                "gameplay.combat_outcome",
            ]
        );
        assert_eq!(
            lines
                .iter()
                .map(|(_, line)| line
                    .split("actor=")
                    .nth(1)
                    .and_then(|actor| actor.split_whitespace().next()))
                .collect::<Vec<_>>(),
            [
                Some("\"Brakka\""),
                Some("\"Brakka\""),
                Some("\"Weasel\""),
                Some("\"Weasel\""),
            ]
        );
    }

    #[test]
    fn story_rest_audit_formatter_quotes_the_actor() {
        assert_eq!(
            story_rest_audit_line("Brakka"),
            "STORY rest actor=\"Brakka\""
        );
    }

    #[test]
    fn story_rest_hit_die_audit_formatter_records_the_spend_result_srd_p187() {
        assert_eq!(
            story_rest_hit_die_audit_line("Brakka", 8, 2, 4, 13),
            "STORY rest actor=\"Brakka\" hit_die=8 modifier=2 healed=4 hit_points=13"
        );
    }

    #[test]
    fn story_say_audit_formatter_contains_only_character_count_and_target() {
        let line = story_say_audit_line("Brakka", 17, "Perrin");
        assert_eq!(
            line,
            "STORY say actor=\"Brakka\" chars=17 target=\"Perrin\""
        );
        assert!(!line.contains("typed words"));
    }

    fn speech(log: &TableLog, text: &str) {
        log.push(
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            text,
        );
    }

    fn composing(log: &Arc<TableLog>) -> Vec<(u32, i32)> {
        attached(log)
            .composing
            .iter()
            .map(|mark| (mark.hero_index, mark.kind))
            .collect()
    }

    #[test]
    fn a_hero_typing_shows_for_everyone_and_stopping_clears_it() {
        let log = TableLog::new();
        let wake = log.subscribe();
        let before = attached(&log).presentation_sequence;
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        assert_eq!(composing(&log), [(0, rpc::ComposeKind::Typing as i32)]);
        assert!(attached(&log).presentation_sequence > before);
        assert!(wake.has_changed().unwrap());
        log.set_composing_at(0, rpc::ComposeKind::Stopped, now);
        assert!(composing(&log).is_empty());
    }

    #[test]
    fn two_heroes_composing_at_once_each_have_their_own_mark() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        log.set_composing_at(1, rpc::ComposeKind::Speaking, now);
        let marks = composing(&log);
        assert!(marks.contains(&(0, rpc::ComposeKind::Typing as i32)));
        assert!(marks.contains(&(1, rpc::ComposeKind::Speaking as i32)));
        log.set_composing_at(0, rpc::ComposeKind::Stopped, now);
        assert_eq!(composing(&log), [(1, rpc::ComposeKind::Speaking as i32)]);
    }

    #[test]
    fn a_refresh_republishes_as_the_displays_heartbeat_and_a_stop_of_nothing_does_not() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        let first = attached(&log).presentation_sequence;
        let mut wake = log.subscribe();
        wake.borrow_and_update();
        log.set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2));
        assert!(attached(&log).presentation_sequence > first);
        assert!(wake.has_changed().unwrap());
        log.set_composing_at(1, rpc::ComposeKind::Stopped, now);
        let settled = attached(&log).presentation_sequence;
        log.set_composing_at(1, rpc::ComposeKind::Stopped, now);
        assert_eq!(attached(&log).presentation_sequence, settled);
    }

    #[test]
    fn a_refresh_keeps_the_mark_alive_and_a_missing_one_expires_it() {
        let log = TableLog::new();
        let now = Instant::now();
        let deadline = log
            .set_composing_at(0, rpc::ComposeKind::Typing, now)
            .expect("a deadline");
        assert_eq!(deadline, now + PRESENCE_EXPIRY);
        // Refreshed at 2 s: alive past the first deadline.
        let refreshed = log
            .set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2))
            .expect("a deadline");
        assert_eq!(log.expire_at(deadline), Some(refreshed));
        assert_eq!(composing(&log).len(), 1);
        // Nothing more arrives: the next expiry clears it and republishes.
        let before = attached(&log).presentation_sequence;
        assert_eq!(log.expire_at(refreshed), None);
        assert!(composing(&log).is_empty());
        assert!(attached(&log).presentation_sequence > before);
    }

    #[test]
    fn unknown_seats_and_a_disabled_log_record_no_presence() {
        let log = TableLog::new();
        let now = Instant::now();
        assert_eq!(log.set_composing_at(2, rpc::ComposeKind::Typing, now), None);
        assert!(composing(&log).is_empty());
        let disabled = TableLog::disabled();
        disabled.set_composing_at(0, rpc::ComposeKind::Typing, now);
        assert!(composing(&disabled).is_empty());
        assert_eq!(attached(&disabled).presentation_sequence, 0);
    }

    #[test]
    fn a_hero_line_clears_that_heros_mark_in_the_same_view() {
        let log = TableLog::new();
        let now = Instant::now();
        log.set_composing_at(0, rpc::ComposeKind::Submitted, now);
        log.set_composing_at(1, rpc::ComposeKind::Typing, now);
        speech(&log, "Who sent you?");
        let view = attached(&log);
        assert_eq!(view.table_log.len(), 1);
        assert_eq!(composing(&log), [(1, rpc::ComposeKind::Typing as i32)]);
    }

    #[test]
    fn a_phones_view_never_carries_or_moves_with_another_players_presence() {
        let log = TableLog::new();
        let now = Instant::now();
        let player = |log: &Arc<TableLog>| {
            let mut view = rpc::ViewMessage {
                audience: Some(rpc::view_message::Audience::Player(
                    rpc::PlayerGameplayView {
                        journey: Some(rpc::JourneyView::default()),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            };
            log.attach(&mut view);
            match view.audience {
                Some(rpc::view_message::Audience::Player(player)) => player.journey.unwrap(),
                _ => unreachable!("the test builds a player view"),
            }
        };
        let before = player(&log).presentation_sequence;
        log.set_composing_at(0, rpc::ComposeKind::Typing, now);
        log.set_composing_at(0, rpc::ComposeKind::Typing, now + Duration::from_secs(2));
        let after = player(&log);
        assert_eq!(after.presentation_sequence, before);
        assert!(after.composing.is_empty());
    }

    #[test]
    fn presence_never_adds_a_table_line_or_a_game_revision() {
        let log = TableLog::new();
        log.set_composing_at(0, rpc::ComposeKind::Typing, Instant::now());
        assert!(log.lines().is_empty());
        let journey = attached(&log);
        assert!(journey.table_log.is_empty());
    }

    #[tokio::test]
    async fn the_reaper_clears_a_mark_nobody_refreshes_and_wakes_the_watchers() {
        let mut log = TableLog::with(true);
        log.expiry = Duration::from_millis(60);
        let log = Arc::new(log);
        let mut wake = log.subscribe();
        log.set_composing(0, rpc::ComposeKind::Typing);
        wake.changed().await.unwrap();
        assert_eq!(composing(&log).len(), 1);
        tokio::time::timeout(Duration::from_secs(2), wake.changed())
            .await
            .expect("woken by the expiry")
            .unwrap();
        assert!(composing(&log).is_empty());
    }

    #[test]
    fn rewind_drops_future_lines_and_reuses_their_sequences() {
        let log = TableLog::new();
        speech(&log, "before");
        let floor = log.position();
        speech(&log, "future one");
        speech(&log, "future two");
        assert_eq!(log.lines().len(), 3);

        log.rewind_to(floor);

        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "before");
        assert_eq!(log.position(), floor);
        speech(&log, "after");
        let lines = log.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].sequence, floor);
        assert_eq!(attached(&log).table_log.len(), 2);
    }

    #[test]
    fn lines_keep_their_order_with_increasing_sequence_and_the_ring_is_bounded() {
        let log = TableLog::new();
        for number in 0..30 {
            speech(&log, &format!("line {number}"));
        }

        let lines = log.lines();

        assert_eq!(lines.len(), RING_LINES);
        assert_eq!(lines[0].text, "line 6");
        assert_eq!(lines[RING_LINES - 1].text, "line 29");
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert_eq!(lines[RING_LINES - 1].sequence, 30);
    }

    #[test]
    fn npc_event_source_must_be_the_exact_latest_public_speech() {
        let log = TableLog::new();
        let source = log
            .push_actor(
                "merchant",
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Here, have a look.",
            )
            .unwrap();

        assert!(log.is_latest_npc_speech(source, "merchant", "Marek", "Here, have a look."));
        assert!(!log.is_latest_npc_speech(source, "other-merchant", "Marek", "Here, have a look."));

        log.push_actor(
            "guard",
            "Runa",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "That is enough.",
        );
        assert!(!log.is_latest_npc_speech(source, "merchant", "Marek", "Here, have a look."));
    }

    #[test]
    fn a_check_is_attributed_to_the_roll_and_preserves_its_outcome() {
        let log = TableLog::new();
        let text = "Brakka searches behind the counter: Wisdom (Perception) 16 + 3 = 19 — success.";
        log.push_check(log.reserve(), 0, text);
        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, "Roll");
        assert_eq!(lines[0].role, rpc::TableRole::Narrator as i32);
        assert_ne!(lines[0].kind, rpc::TableKind::Speech as i32);
        assert_eq!(lines[0].text, text);
    }

    #[test]
    fn a_filled_reservation_sorts_before_a_line_pushed_after_it() {
        let log = TableLog::new();
        let reservation = log.reserve();
        speech(&log, "Calla greets the party");
        log.push_reserved(
            reservation,
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Action,
            "leads on to the bothy",
        );

        let lines = log.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "leads on to the bothy");
        assert_eq!(lines[1].text, "Calla greets the party");
        assert!(lines[0].sequence < lines[1].sequence);
        assert_eq!(attached(&log).table_log, lines);
    }

    #[test]
    fn a_released_reservation_leaves_no_line_and_allows_a_sequence_gap() {
        let log = TableLog::new();
        let reservation = log.reserve();
        drop(reservation);
        speech(&log, "Calla greets the party");

        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].sequence, 2);
        assert_eq!(lines[0].text, "Calla greets the party");
    }

    #[test]
    fn filling_an_old_reservation_keeps_the_ring_size_limit() {
        let log = TableLog::new();
        let reservation = log.reserve();
        for number in 0..30 {
            speech(&log, &format!("line {number}"));
        }
        log.push_reserved(
            reservation,
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Action,
            "leads on to the bothy",
        );

        let lines = log.lines();
        assert_eq!(lines.len(), RING_LINES);
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert_eq!(lines[RING_LINES - 1].sequence, 31);
    }

    fn display_view() -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn attached(log: &Arc<TableLog>) -> rpc::JourneyView {
        let mut view = display_view();
        log.attach(&mut view);
        match view.audience {
            Some(rpc::view_message::Audience::Display(display)) => display.journey.unwrap(),
            _ => unreachable!("the test builds a display view"),
        }
    }

    fn deception_record() -> super::super::freeform_action::CheckRecord {
        super::super::freeform_action::CheckRecord {
            kind: rpc::RollKind::Check,
            subject: "Brakka".to_owned(),
            label: "Charisma (Deception, Advantage: 8 and 1)".to_owned(),
            die: 8,
            modifier: -1,
            total: 7,
            against: "DC",
            number: 15,
            result: "failure",
            success: false,
        }
    }

    #[test]
    fn a_check_row_carries_its_faces_modifier_total_dc_and_verdict_to_the_display() {
        let log = TableLog::new();
        log.push_check(
            log.reserve(),
            1,
            deception_record().row_with_draws(&[8, 1], 1),
        );

        let journey = attached(&log);
        let [line] = journey.table_log.as_slice() else {
            panic!("one roll row: {:?}", journey.table_log);
        };
        assert_eq!(line.speaker, CHECK_SPEAKER);
        assert_eq!(line.hero_index, 1);
        assert_eq!(
            line.text,
            "Brakka: Charisma (Deception, Advantage: 8 and 1) 8 - 1 = 7 vs DC 15 \u{2014} failure."
        );
        assert_eq!(
            line.roll,
            Some(rpc::TableRoll {
                faces: vec![8, 1],
                kept: 8,
                roll_mode: 1,
                modifier: -1,
                total: 7,
                against: rpc::RollAgainst::Dc as i32,
                target_number: 15,
                succeeded: false,
                label: "Brakka \u{b7} Deception".to_owned(),
                kind: rpc::RollKind::Check as i32,
            })
        );
    }

    #[test]
    fn a_plain_roll_is_one_face_and_an_armour_class_roll_says_so() {
        let mut record = deception_record();
        record.label = "Strength (unarmed strike)".to_owned();
        record.against = "AC";
        record.number = 10;
        record.success = true;
        let row = record.row();
        let roll = row.roll.expect("a plain roll is structured");
        assert_eq!(roll.faces, [8]);
        assert_eq!(roll.kept, 8);
        assert_eq!(roll.roll_mode, 0);
        assert_eq!(roll.against, rpc::RollAgainst::Ac as i32);
        assert_eq!(roll.target_number, 10);
        assert!(roll.succeeded);
        assert_eq!(roll.label, "Brakka \u{b7} unarmed strike");
    }

    #[test]
    fn a_text_only_row_carries_no_roll_and_every_roll_row_stays_in_table_order() {
        let log = TableLog::new();
        log.push_check(log.reserve(), 0, "Brynn rolls Persuasion: 12, fails.");
        log.push_check(log.reserve(), 0, deception_record().row());

        let lines = attached(&log).table_log;
        assert_eq!(lines[0].roll, None);
        assert!(lines[1].roll.is_some());
        assert!(lines[0].sequence < lines[1].sequence);
    }

    #[test]
    fn an_ability_check_receipt_becomes_a_structured_row_with_its_skill_label() {
        let check = rpc::AbilityCheckResult {
            die: 12,
            ability_modifier: 3,
            proficiency_bonus: 2,
            total: 17,
            difficulty_class: 14,
            succeeded: true,
            source_revision: "a:b:perception:visible".to_owned(),
            faces: vec![12, 4],
            roll_mode: 1,
        };
        let row = ability_check_row("Brakka searches.".to_owned(), "Brakka", &check);
        let roll = row.roll.expect("a receipt is structured");
        assert_eq!(roll.faces, [12, 4]);
        assert_eq!(roll.kept, 12);
        assert_eq!((roll.modifier, roll.total, roll.target_number), (5, 17, 14));
        assert!(roll.succeeded);
        assert_eq!(roll.label, "Brakka \u{b7} Perception");
    }

    #[test]
    fn roll_labels_name_the_skill_and_survive_unusual_labels() {
        assert_eq!(
            roll_label("Brakka", "Charisma (Deception, Advantage: 8 and 1)"),
            "Brakka \u{b7} Deception"
        );
        assert_eq!(
            roll_label("Mira", "Dexterity save"),
            "Mira \u{b7} Dexterity save"
        );
        assert_eq!(roll_label("Mira", ""), "Mira");
    }

    #[test]
    fn every_change_moves_the_presentation_sequence_and_wakes_watchers() {
        let log = TableLog::new();
        let wake = log.subscribe();
        let first = attached(&log).presentation_sequence;
        speech(&log, "Who sent you?");
        let typed = attached(&log).presentation_sequence;
        let pending = log.reply_pending("Courier");
        let thinking = attached(&log);
        drop(pending);
        let done = attached(&log);

        assert!(first < typed);
        assert!(typed < thinking.presentation_sequence);
        assert!(thinking.presentation_sequence < done.presentation_sequence);
        assert_eq!(thinking.reply_speaker, "Courier");
        assert_eq!(done.reply_speaker, "");
        assert!(wake.has_changed().unwrap());
    }

    // A party note is written to the notebook, not the table feed, yet the phones read the
    // notebook from the view: its write must wake every watcher to publish a fresh view.
    #[test]
    fn a_notebook_write_wakes_watchers_without_adding_a_table_line() {
        let log = TableLog::new();
        let wake = log.subscribe();
        let before = attached(&log);
        log.push(
            "table",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            "NOTE added kind=note title=\"Party note\" scene=opening",
        );
        let after = attached(&log);

        assert!(wake.has_changed().unwrap());
        assert!(after.presentation_sequence > before.presentation_sequence);
        assert!(after.table_log.is_empty());
    }

    #[test]
    fn a_reply_mark_clears_even_when_the_reply_never_comes() {
        let log = TableLog::new();
        {
            let _abandoned = log.reply_pending("Courier");
            assert_eq!(attached(&log).reply_speaker, "Courier");
        }
        assert_eq!(attached(&log).reply_speaker, "");
        let disabled = TableLog::disabled();
        drop(disabled.reply_pending("Courier"));
        assert_eq!(attached(&disabled).presentation_sequence, 0);
    }

    #[test]
    fn text_is_cleaned_cut_and_empty_lines_are_dropped() {
        let log = TableLog::new();

        speech(&log, "  hello\u{0}\nthere\t friend ");
        speech(&log, &"x".repeat(LINE_CHARACTERS + 40));
        speech(&log, " \n ");

        let lines = log.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello there friend");
        assert_eq!(lines[1].text.chars().count(), LINE_CHARACTERS);
    }

    #[test]
    fn a_new_line_wakes_a_watcher_and_a_disabled_log_stays_silent() {
        let log = TableLog::new();
        let mut watcher = log.subscribe();
        assert!(!watcher.has_changed().unwrap());

        speech(&log, "hello");

        assert!(watcher.has_changed().unwrap());
        assert_eq!(*watcher.borrow_and_update(), 1);
        let off = TableLog::disabled();
        speech(&off, "hello");
        assert!(off.lines().is_empty());
    }

    fn story_view(phase: rpc::JourneyPhase, title: &str, display: bool) -> rpc::ViewMessage {
        let journey = Some(rpc::JourneyView {
            phase: phase as i32,
            ..Default::default()
        });
        let scene = Some(rpc::GameplayScene {
            title: title.to_owned(),
            ..Default::default()
        });
        rpc::ViewMessage {
            audience: Some(if display {
                rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
                    journey,
                    scene,
                    ..Default::default()
                })
            } else {
                rpc::view_message::Audience::Player(rpc::PlayerGameplayView {
                    journey,
                    scene,
                    offers: vec![rpc::GameplayActionOffer::default()],
                    action_available: true,
                    ..Default::default()
                })
            }),
            ..Default::default()
        }
    }

    fn offers_after_hold(log: &Arc<TableLog>, mut view: rpc::ViewMessage, now: Instant) -> usize {
        log.apply_story_hold(&mut view, now);
        match view.audience {
            Some(rpc::view_message::Audience::Player(player)) => player.offers.len(),
            _ => 0,
        }
    }

    #[test]
    fn a_new_story_scene_withholds_its_choices_for_a_moment_and_then_gives_them() {
        let log = TableLog::new();
        let start = Instant::now();
        let scene = || story_view(rpc::JourneyPhase::Opening, "The Windward Pass", false);
        let last = start + Duration::from_millis(STORY_HOLD_MS - 1);

        assert_eq!(offers_after_hold(&log, scene(), start), 0, "being told");
        assert_eq!(offers_after_hold(&log, scene(), last), 0, "the other phone");

        // The timer's own path ends the hold and wakes the screens.
        let watcher = log.subscribe();
        let end = start + Duration::from_millis(STORY_HOLD_MS);
        assert_eq!(log.expire_pacing_at(end), None);
        assert!(watcher.has_changed().unwrap());
        assert_eq!(offers_after_hold(&log, scene(), end), 1);

        // The next scene is held in its turn; the one before it is not held again.
        let next = || story_view(rpc::JourneyPhase::Dialogue, "A Soldier's Errand", false);
        assert_eq!(offers_after_hold(&log, next(), end), 0);
        let later = end + Duration::from_millis(STORY_HOLD_MS);
        assert_eq!(offers_after_hold(&log, next(), later), 1);
        assert_eq!(offers_after_hold(&log, scene(), later), 1);
    }

    #[test]
    fn the_story_hold_never_touches_the_lobby_hero_creation_or_the_display() {
        let log = TableLog::new();
        let now = Instant::now();

        for phase in [
            rpc::JourneyPhase::Room,
            rpc::JourneyPhase::CharacterCreation,
        ] {
            let view = story_view(phase, "The Windward Pass", false);
            assert_eq!(offers_after_hold(&log, view, now), 1, "{phase:?}");
        }
        let mut creating = story_view(rpc::JourneyPhase::Opening, "The Windward Pass", false);
        if let Some(rpc::view_message::Audience::Player(player)) = creating.audience.as_mut() {
            player.journey.as_mut().unwrap().creation = Some(Default::default());
        }
        assert_eq!(offers_after_hold(&log, creating, now), 1, "creating a hero");

        let mut display = story_view(rpc::JourneyPhase::Dialogue, "A Soldier's Errand", true);
        let before = display.clone();
        log.apply_story_hold(&mut display, now);
        assert_eq!(display, before);
        // The display's view did not start a hold either: a phone's view still does.
        let phone = story_view(rpc::JourneyPhase::Dialogue, "A Soldier's Errand", false);
        assert_eq!(offers_after_hold(&log, phone, now), 0);
    }

    #[test]
    fn two_views_that_differ_cannot_restart_the_hold_against_each_other() {
        let log = TableLog::new();
        let start = Instant::now();
        let first = || story_view(rpc::JourneyPhase::Dialogue, "A Soldier's Errand", false);
        let second = || story_view(rpc::JourneyPhase::Dialogue, "A Soldier's Errand.", false);
        assert_eq!(offers_after_hold(&log, first(), start), 0);
        assert_eq!(offers_after_hold(&log, second(), start), 0);
        let end = start + Duration::from_millis(2 * STORY_HOLD_MS);
        assert_eq!(log.expire_pacing_at(end), None);
        for _ in 0..3 {
            assert_eq!(offers_after_hold(&log, first(), end), 1);
            assert_eq!(offers_after_hold(&log, second(), end), 1);
        }
    }

    #[test]
    fn public_hero_speech_is_recorded_once_and_wakes_both_audiences() {
        let log = TableLog::new();
        let watcher = log.subscribe();

        log.push_speech("Sigrun", 1, "Brakka, take the left one");

        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, "Sigrun");
        assert_eq!(lines[0].hero_index, 1);
        assert_eq!(lines[0].role, rpc::TableRole::Hero as i32);
        assert_eq!(lines[0].kind, rpc::TableKind::Speech as i32);
        assert_eq!(lines[0].text, "Brakka, take the left one");
        assert!(watcher.has_changed().unwrap());

        let view = attached(&log);
        assert_eq!(view.table_log, lines);
        assert_eq!(view.presentation_sequence, 1);

        let mut player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        log.attach(&mut player);
        let Some(rpc::view_message::Audience::Player(player)) = player.audience else {
            panic!("the test builds a player view");
        };
        assert_eq!(player.journey.unwrap().table_log, lines);
    }

    fn outcome(actor: &str, target: &str, hit: bool, damage: u32) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            hit,
            damage,
            // Still standing after the blow: a hit that leaves 0 is a creature that dropped.
            target_hit_points: 12,
            ..Default::default()
        }
    }

    #[test]
    fn notebook_add_audit_lines_are_not_published_to_the_player_table_log() {
        let log = TableLog::new();
        log.push(
            "table",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            "NOTE added kind=place title=\"the Lantern-Maker's Shop\" scene=The Lantern-Maker's Shop",
        );

        assert!(log.lines().is_empty());

        log.push(
            "Narrator",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            "Added to the notebook: the Lantern-Maker's Shop",
        );
        assert_eq!(
            log.lines().last().map(|line| line.text.as_str()),
            Some("Added to the notebook: the Lantern-Maker's Shop")
        );
    }

    #[test]
    fn thrown_and_shot_attacks_use_their_weapon_action_in_the_table_log() {
        let thrown = rpc::CombatOutcome {
            attack_style: 2,
            weapon: "javelin".to_owned(),
            ..outcome("Brakka", "Snag", true, 9)
        };
        let shot = rpc::CombatOutcome {
            attack_style: 2,
            weapon: "light-crossbow".to_owned(),
            damage_type: "piercing".to_owned(),
            ..outcome("Snag", "Brakka", true, 4)
        };
        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[thrown, shot],
            "greatsword",
            "",
            "Attack",
        );

        assert_eq!(lines[0].text, "throws the javelin \u{2014} hit, 9 damage");
        assert_eq!(
            lines[1].text,
            "shoots at Brakka with the light crossbow \u{2014} hit, 4 piercing damage"
        );
    }

    #[test]
    fn an_attack_is_a_phrase_with_its_visible_result_and_the_enemys_answer_follows() {
        let combat = [
            outcome("Brakka", "Snag", true, 9),
            outcome("Snag", "Brakka", false, 0),
        ];

        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &combat,
            "greatsword",
            "the shop",
            "Greatsword strike",
        );

        assert_eq!(lines[0].speaker, "Brakka");
        assert_eq!(
            lines[0].text,
            "strikes with the greatsword \u{2014} hit, 9 damage"
        );
        assert_eq!(lines[1].speaker, "Snag");
        assert_eq!(lines[1].text, "misses Brakka");
        let miss = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[outcome("Brakka", "Snag", false, 0)],
            "greatsword",
            "",
            "",
        );
        assert_eq!(miss[0].text, "swings the greatsword \u{2014} miss");
    }

    fn reaction(actor: &str, target: &str, hit_points_after: u32) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            kind: rpc::OutcomeKind::OpportunityAttack as i32,
            damage_type: "piercing".to_owned(),
            target_hit_points: hit_points_after,
            ..outcome(actor, target, true, 7)
        }
    }

    fn attack(actor: &str, target: &str, hit: bool) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Attack as i32,
            damage_type: "piercing".to_owned(),
            ..outcome(actor, target, hit, if hit { 5 } else { 0 })
        }
    }

    #[test]
    fn a_reaction_is_told_as_one_and_the_creature_it_drops_says_so() {
        let lines = action_lines(
            rpc::GameplayActionKind::Move,
            "Brakka",
            &[
                reaction("Jessa", "Brakka", 13),
                rpc::CombatOutcome {
                    kind: rpc::OutcomeKind::Move as i32,
                    actor_name: "Brakka".to_owned(),
                    path: vec![
                        rpc::GridCell { col: 0, row: 4 },
                        rpc::GridCell { col: 0, row: 3 },
                    ],
                    cost_ft: 10,
                    ..Default::default()
                },
                reaction("Corin", "Brakka", 0),
            ],
            "greatsword",
            "",
            "Move",
        );

        // The lines follow the server's resolution order: Jessa's reaction came before the
        // move, Corin's after it, and the hero's own line stands at the hero's move.
        let told: Vec<(&str, &str)> = lines
            .iter()
            .map(|line| (line.speaker.as_str(), line.text.as_str()))
            .collect();
        assert_eq!(
            told,
            [
                (
                    "Jessa",
                    "makes an opportunity attack on Brakka \u{2014} hit, 7 piercing damage"
                ),
                ("Brakka", "moves 2 squares (10 ft)"),
                (
                    "Corin",
                    "makes an opportunity attack on Brakka \u{2014} hit, 7 piercing damage, Brakka is down"
                ),
            ]
        );
    }

    #[test]
    fn an_unarmed_strike_is_made_not_struck_with() {
        let hit = rpc::CombatOutcome {
            weapon: "unarmed-strike".to_owned(),
            damage_type: "bludgeoning".to_owned(),
            ..outcome("Brakka", "Corin", true, 4)
        };
        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[hit],
            "unarmed strike",
            "",
            "Attack",
        );
        assert_eq!(
            lines[0].text,
            "makes an unarmed strike \u{2014} hit, 4 bludgeoning damage"
        );
        let missed = rpc::CombatOutcome {
            weapon: "unarmed-strike".to_owned(),
            ..outcome("Brakka", "Corin", false, 0)
        };
        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[missed],
            "unarmed strike",
            "",
            "Attack",
        );
        assert_eq!(lines[0].text, "swings an unarmed strike \u{2014} miss");
    }

    #[test]
    fn a_fight_start_carries_no_offer_label_hero_row() {
        let combat = [
            rpc::CombatOutcome {
                kind: rpc::OutcomeKind::Move as i32,
                actor_name: "Jessa".to_owned(),
                path: vec![rpc::GridCell { col: 0, row: 0 }],
                cost_ft: 5,
                ..Default::default()
            },
            outcome("Jessa", "Brakka", false, 0),
        ];
        let lines = action_lines(
            rpc::GameplayActionKind::SceneAssault,
            "Brakka",
            &combat,
            "greatsword",
            "",
            "Start a fight",
        );
        assert!(
            lines.iter().all(|line| line.speaker != "Brakka"),
            "no hero row from the offer label"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.text.contains("starts a fight")),
            "the offer label is never the hero's line"
        );
        // A single-square move is one square, for heroes and foes alike.
        assert_eq!(lines[0].text, "moves 1 square (5 ft)");
        assert_eq!(lines[1].text, "misses Brakka");
    }

    #[test]
    fn fight_start_narration_reserved_before_the_submission_precedes_its_combat_rows() {
        let log = TableLog::new();
        let held = log.reserve_lines(2);
        // The submission's own combat rows land first in time...
        log.push(
            "Jessa",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Action,
            "moves 1 square (5 ft)",
        );
        // ...but the narration reserved before it fills the earlier sequences.
        for (line, reservation) in [
            "Brakka attacks Corin.",
            "Jessa joins the fight against Brakka.",
        ]
        .into_iter()
        .zip(held)
        {
            log.push_reserved(
                reservation,
                "Narrator",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Action,
                line,
            );
        }
        let rows = log.lines();
        let texts: Vec<&str> = rows.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Brakka attacks Corin.",
                "Jessa joins the fight against Brakka.",
                "moves 1 square (5 ft)",
            ]
        );
    }

    #[test]
    fn multiline_fight_narration_uses_separate_table_rows() {
        let log = TableLog::new();
        let sequences = log.push_lines(
            "Narrator",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            "The fight is over.\nMarek fled the shop.\nCorin stayed out of the fight.",
        );

        assert_eq!(sequences.len(), 3);
        assert_eq!(sequences[1], sequences[0].map(|sequence| sequence + 1));
        assert_eq!(sequences[2], sequences[1].map(|sequence| sequence + 1));
        let rows = log.lines();
        assert_eq!(
            rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>(),
            [
                "The fight is over.",
                "Marek fled the shop.",
                "Corin stayed out of the fight.",
            ]
        );
    }

    #[test]
    fn a_knockout_says_the_creature_falls_unconscious() {
        let knocked = rpc::CombatOutcome {
            knocked_out: true,
            target_hit_points: 1,
            ..attack("Corin", "Perrin", true)
        };
        let lines = action_lines(
            rpc::GameplayActionKind::EndTurn,
            "Sigrun",
            &[knocked],
            "greatsword",
            "",
            "",
        );
        assert_eq!(
            lines[0].text,
            "strikes Perrin \u{2014} hit, 5 piercing damage, Perrin falls unconscious"
        );
    }

    #[test]
    fn a_multiattack_is_one_line_with_every_roll_in_it() {
        let lines = action_lines(
            rpc::GameplayActionKind::EndTurn,
            "Sigrun",
            &[
                attack("Corin", "Sigrun", false),
                attack("Corin", "Sigrun", true),
                attack("Jessa", "Sigrun", true),
            ],
            "greatsword",
            "",
            "",
        );
        let told: Vec<(&str, &str)> = lines
            .iter()
            .map(|line| (line.speaker.as_str(), line.text.as_str()))
            .collect();
        assert_eq!(
            told,
            [
                (
                    "Corin",
                    "makes two attacks on Sigrun \u{2014} miss; hit, 5 piercing damage"
                ),
                ("Jessa", "strikes Sigrun \u{2014} hit, 5 piercing damage"),
            ]
        );
    }

    #[test]
    fn a_death_save_is_told_once_as_the_heros_roll_with_its_die_attached() {
        let save = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::DeathSave as i32,
            actor_name: "Brakka".to_owned(),
            attack_die: 16,
            death_save_result: rpc::DeathSaveResult::Success as i32,
            death_successes: 1,
            ..Default::default()
        };
        // The same save handed over twice is still one line.
        let lines = action_lines(
            rpc::GameplayActionKind::EndTurn,
            "Sigrun",
            &[save.clone(), save],
            "greatsword",
            "",
            "",
        );
        let saves: Vec<&ActionLine> = lines
            .iter()
            .filter(|line| line.speaker == "Brakka")
            .collect();
        assert_eq!(saves.len(), 1);
        assert_eq!(
            saves[0].text,
            "makes a Death Saving Throw: 16 \u{2014} success (1 success, 0 failures)"
        );
        let roll = saves[0].roll.as_ref().expect("the die rides on the line");
        assert_eq!(roll.kind, rpc::RollKind::DeathSave as i32);
        assert_eq!(roll.kept, 16);
        assert!(roll.succeeded);
    }

    #[test]
    fn a_stabilise_check_line_carries_the_medicine_die() {
        let stabilise = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Stabilisation as i32,
            actor_name: "Sigrun".to_owned(),
            target_name: "Brakka".to_owned(),
            stabilization: Some(rpc::StabilizationResult {
                method: rpc::StabilizationMethod::Medicine as i32,
                die: 13,
                wisdom_modifier: 1,
                dc: 10,
                succeeded: true,
                uses_left: 0,
            }),
            ..Default::default()
        };
        let lines = action_lines(
            rpc::GameplayActionKind::HelpAttack,
            "Sigrun",
            &[stabilise],
            "greatsword",
            "",
            "",
        );
        let roll = lines[0].roll.as_ref().expect("the hero's check is a roll");
        assert_eq!(roll.label, "Sigrun \u{b7} Medicine (stabilize Brakka)");
        assert_eq!((roll.kept, roll.modifier, roll.total), (13, 1, 14));
        assert_eq!(roll.kind, rpc::RollKind::Stabilize as i32);
    }

    #[test]
    fn a_hit_names_the_damage_type_the_rules_gave_it() {
        let own = rpc::CombatOutcome {
            damage_type: "slashing".to_owned(),
            critical: true,
            ..outcome("Brakka", "Snag", true, 9)
        };
        let answer = rpc::CombatOutcome {
            damage_type: "piercing".to_owned(),
            ..outcome("Snag", "Brakka", true, 4)
        };

        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[own, answer],
            "greatsword",
            "",
            "",
        );

        assert_eq!(
            lines[0].text,
            "strikes with the greatsword \u{2014} critical hit, 9 slashing damage"
        );
        assert_eq!(
            lines[1].text,
            "strikes Brakka \u{2014} hit, 4 piercing damage"
        );
    }

    #[test]
    fn a_grazing_greatsword_attack_names_its_damage_instead_of_a_miss() {
        let graze = rpc::CombatOutcome {
            grazed: true,
            damage: 3,
            damage_type: "slashing".to_owned(),
            weapon: "greatsword".to_owned(),
            ..outcome("Brakka", "Snag", false, 3)
        };

        let lines = action_lines(
            rpc::GameplayActionKind::GreatswordAttack,
            "Brakka",
            &[graze],
            "greatsword",
            "",
            "",
        );

        assert_eq!(
            lines[0].text,
            "swings the greatsword \u{2014} graze, 3 slashing damage"
        );
    }

    #[test]
    fn a_resolved_ranged_attack_uses_shot_wording_for_any_attack_kind() {
        let shot = rpc::CombatOutcome {
            attack_style: 2,
            weapon: "longbow".to_owned(),
            damage_type: "piercing".to_owned(),
            ..outcome("Sigrun", "Snag", true, 10)
        };

        let lines = action_lines(
            rpc::GameplayActionKind::OpportunityAttack,
            "Sigrun",
            &[shot],
            "shortsword",
            "",
            "Strikes with the longbow",
        );

        assert_eq!(
            lines[0].text,
            "makes an opportunity attack on Snag: shoots the longbow \u{2014} hit, 10 piercing damage"
        );
    }

    #[test]
    fn combat_wording_is_present_in_the_player_facing_table_rows() {
        let log = TableLog::new();
        let mut opportunity = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::OpportunityAttack as i32,
            weapon: "greatsword".to_owned(),
            damage_type: "slashing".to_owned(),
            ..outcome("Brakka", "Corin", true, 8)
        };
        opportunity.critical = true;
        let attack_lines = action_lines(
            rpc::GameplayActionKind::OpportunityAttack,
            "Brakka",
            &[opportunity],
            "greatsword",
            "",
            "Opportunity attack",
        );
        for line in &attack_lines {
            log.push_action_line(None, line, rpc::TableRole::Hero, 0);
        }

        let journey = attached(&log);
        assert_eq!(
            journey.table_log[0].text,
            "makes an opportunity attack on Corin: strikes with the greatsword \u{2014} critical hit, 8 slashing damage"
        );
    }

    #[test]
    fn combat_table_rows_use_critical_hit_singular_movement_and_correct_save_counts() {
        let log = TableLog::new();
        let critical = rpc::CombatOutcome {
            critical: true,
            damage_type: "piercing".to_owned(),
            ..attack("Corin", "Brakka", true)
        };
        let movement = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Move as i32,
            actor_name: "Jessa".to_owned(),
            path: vec![rpc::GridCell { col: 1, row: 2 }],
            cost_ft: 5,
            ..Default::default()
        };
        let save = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::DeathSave as i32,
            actor_name: "Brakka".to_owned(),
            attack_die: 4,
            death_save_result: rpc::DeathSaveResult::Failure as i32,
            death_failures: 1,
            ..Default::default()
        };
        for line in action_lines(
            rpc::GameplayActionKind::EndTurn,
            "Sigrun",
            &[critical, movement, save],
            "",
            "",
            "End Turn",
        ) {
            log.push_action_line(None, &line, rpc::TableRole::Hero, 0);
        }

        let rows = attached(&log).table_log;
        let text: Vec<&str> = rows.iter().map(|line| line.text.as_str()).collect();
        assert!(text.contains(&"strikes Brakka — critical hit, 5 piercing damage"));
        assert!(text.contains(&"moves 1 square (5 ft)"));
        assert!(text.contains(&"makes a Death Saving Throw: 4 — failure (0 successes, 1 failure)"));
    }

    #[test]
    fn stable_recovery_is_narration_about_the_hero_rather_than_the_heros_speech() {
        let recovery = rpc::CombatOutcome {
            kind: rpc::OutcomeKind::Recovery as i32,
            actor_name: "Sigrun".to_owned(),
            recovery_hours: 1,
            ..Default::default()
        };
        let lines = action_lines(
            rpc::GameplayActionKind::EndTurn,
            "Brakka",
            &[recovery],
            "greatsword",
            "",
            "End turn",
        );
        let line = lines
            .iter()
            .find(|line| line.text.contains("regains 1 hit point"))
            .unwrap();
        assert_eq!(line.speaker, "Narrator");
        assert_eq!(
            line.text,
            "Sigrun has been stable for 1 hour and regains 1 hit point"
        );
    }

    #[test]
    fn a_hero_move_sums_its_resolved_segments_into_one_distance_line() {
        let segments = (0..4)
            .map(|_| rpc::CombatOutcome {
                kind: rpc::OutcomeKind::Move as i32,
                path: vec![rpc::GridCell::default()],
                cost_ft: 5,
                ..outcome("Brakka", "", false, 0)
            })
            .collect::<Vec<_>>();

        let lines = action_lines(
            rpc::GameplayActionKind::Move,
            "Brakka",
            &segments,
            "greatsword",
            "",
            "Move",
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "moves 4 squares (20 ft)");
    }

    #[test]
    fn other_actions_use_fixed_or_label_phrases_and_creating_a_hero_is_silent() {
        let phrase = |kind, label: &str| {
            action_lines(
                kind,
                "Brakka",
                &[],
                "greatsword",
                "the Archivist's Study",
                label,
            )
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
        };
        use rpc::GameplayActionKind as Kind;
        assert_eq!(phrase(Kind::ShortRest, "x"), ["takes a short rest"]);
        assert_eq!(
            phrase(Kind::ChooseHarborScene, "x"),
            ["leads on to the Archivist's Study"]
        );
        assert_eq!(
            phrase(Kind::AskCourier, "Question Ilsa closely"),
            ["questions Ilsa closely"]
        );
        assert!(phrase(Kind::CreateCharacter, "Bring your hero to life").is_empty());
        assert!(phrase(Kind::BeginStory, "Enter the shop").is_empty());
        assert!(phrase(Kind::EndTurn, "End turn").is_empty());
    }

    #[test]
    fn both_audiences_receive_the_feed_and_a_disabled_log_attaches_nothing() {
        let log = TableLog::new();
        speech(&log, "Hello there");
        let player = || rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        };
        let display = || rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
        };
        let feed = |view: &rpc::ViewMessage| match &view.audience {
            Some(rpc::view_message::Audience::Player(p)) => {
                p.journey.as_ref().unwrap().table_log.len()
            }
            Some(rpc::view_message::Audience::Display(d)) => {
                d.journey.as_ref().unwrap().table_log.len()
            }
            None => 0,
        };

        let (mut for_player, mut for_display, mut untouched) = (player(), display(), player());
        log.attach(&mut for_player);
        log.attach(&mut for_display);
        TableLog::disabled().attach(&mut untouched);

        assert_eq!((feed(&for_player), feed(&for_display)), (1, 1));
        assert_eq!(feed(&untouched), 0);
    }

    #[test]
    fn errand_choices_use_actions_and_unconvertible_titles_stay_quoted() {
        let phrase = |label: &str| {
            action_lines(
                rpc::GameplayActionKind::ResolveErrand,
                "Brakka",
                &[],
                "greatsword",
                "",
                label,
            )[0]
            .text
            .clone()
        };
        assert_eq!(
            phrase("Settle things with Wystan"),
            "settles things with Wystan"
        );
        assert_eq!(phrase("Close the chapter"), "closes the chapter");
        assert_eq!(
            phrase("The Road Takes Its Toll"),
            "chose \"The Road Takes Its Toll\""
        );
        assert_eq!(phrase("Calla's Thanks"), "chose \"Calla's Thanks\"");
        assert_eq!(third_person("Ask Halden plainly"), "asks Halden plainly");
        assert_eq!(third_person("Bar the thief's way"), "bars the thief's way");
    }

    #[test]
    fn labels_become_third_person_phrases() {
        assert_eq!(
            third_person("Ask Ilsa what is wrong"),
            "asks Ilsa what is wrong"
        );
        assert_eq!(
            third_person("Press on to the boathouse"),
            "presses on to the boathouse"
        );
        assert_eq!(
            third_person("Vow to see Ilsa through"),
            "vows to see Ilsa through"
        );
        assert_eq!(third_person("Go"), "goes");
    }

    /// Every label the tables can produce, read from the same files the server loads.
    fn table_labels() -> Vec<String> {
        let scripts: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../content/adventure-tables/beat-scripts.json"
        ))
        .unwrap();
        let types: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../content/adventure-tables/chapter-types.json"
        ))
        .unwrap();
        let mut labels = Vec::new();
        for template in scripts["chapter_types"].as_object().unwrap().values() {
            // `labels` is optional in a template; a template without them has none to check.
            for label in template["script"]["labels"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
            {
                labels.push(label.as_str().unwrap().to_owned());
            }
        }
        fn choices(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, item) in map {
                        if matches!(key.as_str(), "asks" | "helps" | "stands") {
                            if let Some(list) = item.as_array() {
                                out.extend(
                                    list.iter().filter_map(|l| l.as_str().map(str::to_owned)),
                                );
                            }
                        }
                        choices(item, out);
                    }
                }
                serde_json::Value::Array(list) => list.iter().for_each(|item| choices(item, out)),
                _ => {}
            }
        }
        choices(&types, &mut labels);
        labels
    }

    #[test]
    fn every_table_label_conjugates_to_its_correct_third_person_form() {
        let labels = table_labels();
        assert!(labels.len() > 50, "the tables supply the labels under test");
        for label in &labels {
            let verb = label.split(' ').next().unwrap().to_lowercase();
            let third = third_person(label);
            let action = action_lines(
                rpc::GameplayActionKind::ResolveErrand,
                "Brakka",
                &[],
                "greatsword",
                "",
                label,
            )[0]
            .text
            .clone();
            assert_eq!(action, third, "choice label {label:?}");
            let form = third.split(' ').next().unwrap();
            let expected = if verb.ends_with("y")
                && !verb[..verb.len() - 1].ends_with(['a', 'e', 'i', 'o', 'u'])
            {
                format!("{}ies", &verb[..verb.len() - 1])
            } else if verb.ends_with(['s', 'x', 'z', 'o'])
                || verb.ends_with("ch")
                || verb.ends_with("sh")
            {
                format!("{verb}es")
            } else {
                format!("{verb}s")
            };
            assert_eq!(form, expected, "label {label:?}");
            let words: Vec<&str> = third.split(' ').collect();
            assert!(
                !words
                    .iter()
                    .any(|word| matches!(*word, "your" | "you" | "yourself")),
                "second person left in {third:?}"
            );
        }
        assert_eq!(
            third_person("Carry the clue onward"),
            "carries the clue onward"
        );
        assert_eq!(third_person("Study the markings"), "studies the markings");
        assert_eq!(third_person("Buy the ticket"), "buys the ticket");
        assert_eq!(third_person("Rally the crowd"), "rallies the crowd");
        assert_eq!(
            third_person("Pledge your help to Kestrel"),
            "pledges their help to Kestrel"
        );
        assert_eq!(third_person("Trust you"), "trusts them");
    }

    fn fight_view(player_audience: bool, latest: u64, beats: &[(u64, bool)]) -> rpc::ViewMessage {
        let outcomes = beats
            .iter()
            .map(|(sequence, knocked_out)| rpc::CombatOutcome {
                sequence: *sequence,
                knocked_out: *knocked_out,
                kind: rpc::OutcomeKind::Attack as i32,
                ..Default::default()
            })
            .collect();
        let journey = rpc::JourneyView {
            combat: Some(rpc::CombatView {
                outcomes,
                latest_sequence: latest,
                ..Default::default()
            }),
            ..Default::default()
        };
        let audience = if player_audience {
            rpc::view_message::Audience::Player(rpc::PlayerGameplayView {
                action_available: true,
                offers: vec![rpc::GameplayActionOffer::default()],
                journey: Some(journey),
                ..Default::default()
            })
        } else {
            rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
                journey: Some(journey),
                ..Default::default()
            })
        };
        rpc::ViewMessage {
            audience: Some(audience),
            ..Default::default()
        }
    }

    fn player_offers(view: &rpc::ViewMessage) -> (bool, usize, u64, u64) {
        match &view.audience {
            Some(rpc::view_message::Audience::Player(player)) => {
                let combat = player.journey.as_ref().unwrap().combat.as_ref().unwrap();
                (
                    player.action_available,
                    player.offers.len(),
                    combat.released_sequence,
                    combat.latest_sequence,
                )
            }
            _ => unreachable!("the test builds a player view"),
        }
    }

    #[test]
    fn pacing_holds_phone_offers_until_the_beats_have_been_presented() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 1_800), (2, 1_800)];
        // Legal walk: CaughtUp -> Presenting on new beats; still held mid-wait.
        let (released, arm) = log.pacing_observe(2, &durations, now);
        assert_eq!(released, 0);
        assert_eq!(arm, Some(now + Duration::from_millis(3_600)));
        let (released, _) = log.pacing_observe(2, &durations, now + Duration::from_millis(1_000));
        assert_eq!(released, 0, "no early release");
        // Illegal: expiring before the release instant releases nothing.
        assert_eq!(
            log.expire_pacing_at(now + Duration::from_millis(3_599)),
            arm
        );
        // Legal: the due release lands and the gate is caught up at the newest beat.
        assert_eq!(
            log.expire_pacing_at(now + Duration::from_millis(3_600)),
            None
        );
        let (released, arm) = log.pacing_observe(2, &durations, now + Duration::from_millis(3_600));
        assert_eq!((released, arm), (2, None));
    }

    #[test]
    fn pacing_extends_for_beats_committed_during_the_wait_but_is_bounded() {
        let log = TableLog::new();
        let now = Instant::now();
        let (released, _) = log.pacing_observe(1, &[(1, 10_000)], now);
        assert_eq!(released, 0);
        let (_, arm) = log.pacing_observe(3, &[(2, 10_000), (3, 10_000)], now);
        assert_eq!(
            arm,
            Some(now + Duration::from_millis(PACING_MAX_WAIT_MS)),
            "one stamped wait never exceeds the bound"
        );
        // A new fight restarts sequences; the gate resets instead of gating forever.
        let (released, _) = log.pacing_observe(1, &[(1, 1_800)], now);
        assert_eq!(released, 0);
        assert_eq!(
            log.expire_pacing_at(now + Duration::from_millis(1_800)),
            None
        );
        let (released, _) =
            log.pacing_observe(1, &[(1, 1_800)], now + Duration::from_millis(1_800));
        assert_eq!(released, 1);
    }

    #[test]
    fn one_displays_reports_release_each_beat_as_it_is_presented() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 9_000), (2, 9_000), (3, 9_000)];
        let (released, _) = log.pacing_observe(3, &durations, now);
        assert_eq!(released, 0);
        // Only the display's newest report speaks for it: its own earlier reports
        // never hold the release back.
        assert_eq!(log.report_presented_at(7, 1, now), 1);
        assert_eq!(
            log.report_presented_at(7, 2, now + Duration::from_millis(100)),
            2
        );
        assert_eq!(
            log.report_presented_at(7, 3, now + Duration::from_millis(200)),
            3,
            "the last beat releases at once, long before the timer bound"
        );
        let (released, arm) = log.pacing_observe(3, &durations, now + Duration::from_millis(200));
        assert_eq!((released, arm), (3, None), "caught up by ack");
        // A report with nothing presenting changes nothing.
        assert_eq!(
            log.report_presented_at(7, 3, now + Duration::from_millis(300)),
            3
        );
    }

    #[test]
    fn two_displays_the_slowest_governs_until_it_stops_reporting() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 9_000), (2, 9_000), (3, 9_000)];
        let (released, _) = log.pacing_observe(3, &durations, now);
        assert_eq!(released, 0);
        // Display 1 is slow, display 2 races ahead: the slowest governs.
        assert_eq!(log.report_presented_at(1, 1, now), 1);
        assert_eq!(
            log.report_presented_at(2, 3, now + Duration::from_millis(400)),
            1,
            "the slow display still governs"
        );
        // Illegal reports are clamped, never obeyed past the pending beats.
        assert_eq!(
            log.report_presented_at(2, 99, now + Duration::from_millis(500)),
            1
        );
        // Display 1 reports nothing more. At the timer's wake just past its bound it
        // is dropped without any new report, and display 2's fresh report releases.
        let fast_fresh = now + Duration::from_millis(ACK_STALE_MS - 500);
        assert_eq!(log.report_presented_at(2, 3, fast_fresh), 1);
        let wake = now + Duration::from_millis(ACK_STALE_MS + 1);
        assert_eq!(log.expire_pacing_at(wake), None);
        assert_eq!(
            log.released_sequence(),
            3,
            "a silent display stopped governing after the bounded time"
        );
    }

    #[test]
    fn a_display_reconnecting_mid_beat_never_moves_the_release_backward() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 9_000), (2, 9_000), (3, 9_000), (4, 9_000), (5, 9_000)];
        let (released, _) = log.pacing_observe(5, &durations, now);
        assert_eq!(released, 0);
        assert_eq!(log.report_presented_at(1, 2, now), 2);
        // The same television reloads: a new instance starts from nothing. It holds
        // further release while it catches up but cannot un-release beat 2.
        let mid = now + Duration::from_millis(300);
        assert_eq!(log.report_presented_at(9, 0, mid), 2);
        assert_eq!(
            log.report_presented_at(1, 4, mid + Duration::from_millis(100)),
            2,
            "the reconnected display governs until it catches up"
        );
        // It jumps to the newest beat (its coherent catch-up) and releases follow.
        assert_eq!(
            log.report_presented_at(9, 5, mid + Duration::from_millis(200)),
            4
        );
        assert_eq!(
            log.report_presented_at(1, 5, mid + Duration::from_millis(300)),
            5
        );
    }

    #[test]
    fn a_display_that_goes_away_leaves_the_timer_bound_in_charge() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 5_000), (2, 5_000)];
        let (released, arm) = log.pacing_observe(2, &durations, now);
        assert_eq!(released, 0);
        assert_eq!(arm, Some(now + Duration::from_millis(10_000)));
        // The only display reports once and is then closed.
        assert_eq!(log.report_presented_at(4, 1, now), 1);
        // Its stale wake drops it but releases nothing more; the timer bound governs.
        let stale_wake = now + Duration::from_millis(ACK_STALE_MS + 1);
        assert_eq!(
            log.expire_pacing_at(stale_wake),
            Some(now + Duration::from_millis(10_000))
        );
        assert_eq!(log.released_sequence(), 1);
        assert_eq!(
            log.expire_pacing_at(now + Duration::from_millis(10_000)),
            None
        );
        assert_eq!(log.released_sequence(), 2);
    }

    #[test]
    fn with_no_display_report_the_timer_bound_alone_releases() {
        let log = TableLog::new();
        let now = Instant::now();
        let durations = [(1, 60_000)];
        let (released, arm) = log.pacing_observe(1, &durations, now);
        assert_eq!(released, 0);
        assert_eq!(
            arm,
            Some(now + Duration::from_millis(PACING_MAX_WAIT_MS)),
            "the bound, not the nominal sum, caps a silent table"
        );
        assert_eq!(
            log.expire_pacing_at(now + Duration::from_millis(PACING_MAX_WAIT_MS)),
            None
        );
        let (released, _) = log.pacing_observe(
            1,
            &durations,
            now + Duration::from_millis(PACING_MAX_WAIT_MS),
        );
        assert_eq!(released, 1);
    }

    #[tokio::test]
    async fn the_pacing_timer_armed_from_the_owner_thread_still_releases() {
        // View publishing runs on the session owner thread, which has no async runtime:
        // arming there must use the handle captured by an earlier watch subscription, or
        // the bound would never fire and the phones would wait forever.
        let log = TableLog::new();
        let mut wake = log.subscribe();
        let now = Instant::now();
        let (released, arm) = log.pacing_observe(1, &[(1, 50)], now);
        assert_eq!(released, 0);
        let arm = arm.expect("a presenting gate arms its bound");
        let owner = log.clone();
        std::thread::spawn(move || owner.arm_pacing(arm))
            .join()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), wake.changed())
            .await
            .expect("woken by the pacing bound")
            .unwrap();
        assert_eq!(log.released_sequence(), 1);
    }

    #[tokio::test]
    async fn a_wait_entered_before_any_runtime_handle_is_rearmed_by_the_first_watcher() {
        // The fight-a5 stall: the gate was entered on the session owner thread, which
        // has no runtime, before any watcher had subscribed, so no timer ever armed.
        // The first subscription must capture the runtime and arm the pending wait.
        let log = std::thread::spawn(|| {
            let log = TableLog::new();
            let (_released, arm) = log.pacing_observe(1, &[(1, 50)], Instant::now());
            // Arming here cannot work: this thread has no runtime and none is captured.
            log.arm_pacing(arm.expect("a presenting gate arms its bound"));
            log
        })
        .join()
        .unwrap();
        assert_eq!(log.released_sequence(), 0);
        let mut wake = log.subscribe();
        tokio::time::timeout(Duration::from_secs(2), wake.changed())
            .await
            .expect("woken by the re-armed pacing bound")
            .unwrap();
        assert_eq!(log.released_sequence(), 1);
    }

    #[test]
    fn attach_withholds_a_players_offers_while_presenting_and_never_gates_the_display() {
        let log = TableLog::new();
        let mut view = fight_view(true, 2, &[(1, false), (2, true)]);
        log.attach(&mut view);
        assert_eq!(
            player_offers(&view),
            (false, 0, 0, 2),
            "resolving: no offers, released behind latest"
        );
        // The display carries the same released stamp but nothing is stripped from it.
        let mut display = fight_view(false, 2, &[(1, false), (2, true)]);
        log.attach(&mut display);
        match &display.audience {
            Some(rpc::view_message::Audience::Display(display)) => {
                let combat = display.journey.as_ref().unwrap().combat.as_ref().unwrap();
                assert_eq!(combat.released_sequence, 0);
                assert_eq!(combat.outcomes.len(), 2);
            }
            _ => unreachable!("the test builds a display view"),
        }
        // A view with no unpresented beats is offered at once.
        let settled = TableLog::new();
        let mut view = fight_view(true, 0, &[]);
        settled.attach(&mut view);
        assert_eq!(player_offers(&view), (true, 1, 0, 0));
    }
}
