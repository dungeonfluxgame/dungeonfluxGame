//! The public table log: the latest lines of who said and did what, shown to every screen.
//!
//! Everything here is table talk: an accepted action's public phrase, the words a hero types to
//! the scene's speaker, and the speaker's in-character reply. The private clue and any other
//! member-private text are never written here by any caller. The log is a bounded in-memory ring
//! per run; it is presentation only and never an authority for game state.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use tokio::sync::watch;

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
/// Displays whose reports are tracked at once; beyond this the stalest report is
/// dropped. Several televisions on one room stay distinguishable by instance.
const ACK_DISPLAYS: usize = 8;

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
                acks: BTreeMap::new(),
            }),
            wake: watch::channel(0).0,
            // Construction usually runs inside the server's runtime; capture its handle
            // now so the first timer can arm even if it is requested (on the session
            // owner thread) before any watcher subscribes.
            runtime: Mutex::new(tokio::runtime::Handle::try_current().ok()),
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
        let Pacing::Presenting { release_at, .. } = ring.pacing else {
            return None;
        };
        let stale = Duration::from_millis(ACK_STALE_MS);
        let ages_out = ring.acks.values().map(|(_, at)| *at + stale).min();
        Some(ages_out.map_or(release_at, |at| at.min(release_at)))
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
            ring.version += 1;
            ring.version + ring.presence_version
        };
        self.wake.send_replace(version);
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

    /// A table setting changed: every screen gets one fresh view, though no line was said and the
    /// game revision did not move. The version it bumps is the one each view carries.
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
    ) {
        self.push_line(None, speaker, role, hero_index, kind, text);
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
    ) {
        self.push_line(reservation.sequence, speaker, role, hero_index, kind, text);
    }

    fn push_line(
        &self,
        reserved_sequence: Option<u64>,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) {
        if !self.enabled {
            return;
        }
        let text: String = text
            .chars()
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
            .collect();
        if text.is_empty() {
            return;
        }
        let version = {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            let sequence = reserved_sequence.unwrap_or_else(|| {
                let sequence = ring.next_sequence;
                ring.next_sequence += 1;
                sequence
            });
            let position = ring.lines.partition_point(|line| line.sequence < sequence);
            ring.lines.insert(
                position,
                rpc::TableLine {
                    sequence,
                    speaker: speaker.chars().take(48).collect(),
                    role: role as i32,
                    hero_index,
                    kind: kind as i32,
                    text,
                },
            );
            while ring.lines.len() > RING_LINES {
                ring.lines.pop_front();
            }
            // The hero's line replaces the hero's loader in the same view.
            if role == rpc::TableRole::Hero && kind == rpc::TableKind::Speech {
                ring.composing.retain(|mark| mark.hero_index != hero_index);
            }
            ring.version += 1;
            ring.version
        };
        self.wake.send_replace(version);
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
            let advanced = Self::advance_pacing_locked(&mut ring, now);
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
}

/// The public lines of an accepted action: the acting hero's short phrase from the offer's
/// label and visible result, then what the enemy did in answer. Creating a hero says nothing.
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
    // Only an attack action is worded from the attack it resolved: a turn that ends, or a move,
    // keeps its own line even when the same submit carries a reaction's blow.
    let attacking = matches!(kind, Kind::GreatswordAttack | Kind::OpportunityAttack);
    let phrase = if let Some(outcome) = own.filter(|_| attacking) {
        let attack = if outcome.attack_style == 2 {
            format!("shoots the {used}")
        } else if outcome.grazed || !outcome.hit {
            format!("swings the {used}")
        } else {
            format!("strikes with the {used}")
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
        format!("{attack} \u{2014} {result}")
    } else {
        match kind {
            Kind::GreatswordAttack => format!("strikes with the {used}"),
            Kind::SecondWind => "catches a second wind".to_owned(),
            Kind::EndTurn => "ends the turn".to_owned(),
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
                format!("moves {squares} squares ({feet} ft)")
            }
            Kind::ChooseHarborScene => format!("leads on to {destination}"),
            Kind::ResolveErrand => format!("chose \"{label}\""),
            _ => third_person(label),
        }
    };
    let mut lines = vec![ActionLine {
        speaker: hero.to_owned(),
        text: phrase,
    }];
    for outcome in combat {
        match rpc::OutcomeKind::try_from(outcome.kind) {
            Ok(rpc::OutcomeKind::DeathSave) => {
                lines.push(ActionLine {
                    speaker: outcome.actor_name.clone(),
                    text: death_save_text(outcome),
                });
                continue;
            }
            Ok(rpc::OutcomeKind::Recovery) => {
                lines.push(ActionLine {
                    speaker: outcome.actor_name.clone(),
                    text: format!(
                        "has been stable for {} hours and regains 1 hit point",
                        outcome.recovery_hours
                    ),
                });
                continue;
            }
            Ok(rpc::OutcomeKind::Move) => {
                if outcome.actor_name != hero {
                    lines.push(ActionLine {
                        speaker: outcome.actor_name.clone(),
                        text: format!(
                            "moves {} squares ({} ft)",
                            outcome.path.len(),
                            outcome.cost_ft
                        ),
                    });
                }
                continue;
            }
            _ => {}
        }
        if outcome.actor_name == hero {
            continue;
        }
        let text = if outcome.hit {
            format!(
                "strikes {} \u{2014} hit, {} damage{}",
                outcome.target_name,
                damage_words(outcome),
                if outcome.knocked_out {
                    format!(", {} falls unconscious", outcome.target_name)
                } else {
                    String::new()
                }
            )
        } else {
            format!("misses {}", outcome.target_name)
        };
        lines.push(ActionLine {
            speaker: outcome.actor_name.clone(),
            text,
        });
    }
    lines
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
            "COMBAT {} actor={:?} target={:?} weapon={} mode={} distance_ft={} cover={} roll_mode={} faces={:?} modifier={} hit={} critical={} damage={}{} reasons={:?}",
            if kind == Ok(rpc::OutcomeKind::OpportunityAttack) {
                "opportunity_attack"
            } else {
                "attack"
            },
            outcome.actor_name,
            outcome.target_name,
            outcome.weapon,
            if outcome.attack_style == 2 {
                "ranged"
            } else {
                "melee"
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
    format!(
        "makes a Death Saving Throw: {} \u{2014} {result} ({} successes, {} failures)",
        outcome.attack_die, outcome.death_successes, outcome.death_failures
    )
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

    fn outcome(actor: &str, target: &str, hit: bool, damage: u32) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            hit,
            damage,
            ..Default::default()
        }
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
            "shoots the longbow \u{2014} hit, 10 piercing damage"
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
    fn ending_labels_are_quoted_and_imperative_labels_are_conjugated() {
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
