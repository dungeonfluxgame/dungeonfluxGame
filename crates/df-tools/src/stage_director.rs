//! The display's cue sequencer: WHEN each change between views is shown, outside of speech.
//!
//! `presentation` works out what changed. This director orders those changes into a FIFO of
//! cues and plays them one at a time, so a killing blow is seen before the scene moves on and a
//! scene change always leaves, shows the interstitial and enters. It is pure and
//! presentation-only: it decides no outcome, sends nothing, and the newest accepted view is
//! always what its `Render` command draws. There is no clock inside; callers pass `now_ms`, so
//! tests use a controlled one.
//!
//! Speech is not sequenced here. The conductor (`stage_conductor`) owns the scene, line, clip
//! and actor machines: it holds a scene change back until the line being spoken has finished and
//! hands it to this director only when the scene is ready to leave. What stays immediate is
//! handled by the conductor too: it fades the line in progress before it passes an outcome (dice,
//! a defeat, the banner, a fight's start) here.
//!
//! Bounds: every state has a maximum dwell, so a finish signal that never arrives times out;
//! when the queue is long or the display is far behind, cosmetic cues are dropped and the
//! outcome-bearing ones (the killing blow, the defeat, the banner) are kept in shortened form.
use std::collections::VecDeque;

/// Queue length beyond which stale cosmetic cues are collapsed.
pub(crate) const QUEUE_CAP: usize = 10;
/// Age of the oldest queued cue beyond which stale cosmetic cues are collapsed.
pub(crate) const LAG_LIMIT_MS: u64 = 4_000;
const SHORT_FLOOR_MS: u32 = 150;
/// How long a fallen character is held on screen with the final roll visible, so the table
/// registers the blow before the scene moves on.
const DEFEAT_HOLD_MS: u32 = 2_500;
/// The title card is about three seconds; the closing card stays a little longer.
const TITLE_CARD_MS: u32 = 3_000;
const CLOSING_CARD_MS: u32 = 4_500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StageState {
    Idle,
    LeavingScene,
    /// The wordmark shows while the painting changes underneath.
    Interstitial,
    /// The adventure's title card over the dimmed painting, before the story's first scene.
    TitleCard,
    EnteringScene,
    Rolling,
    Impact,
    Defeat,
    Healing,
    Aftermath,
    /// The closing card that names the heroes once a fight is won or lost.
    ClosingCard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CueKind {
    /// Draw the newest accepted view now (instant).
    Render,
    /// Leave the scene, show the interstitial (drawing the newest view), enter.
    Scene,
    Roll,
    Impact,
    Defeat,
    Heal,
    Banner,
    /// Only ever started by a banner: the closing card follows the aftermath.
    Closing,
}

#[derive(Clone, Debug)]
pub(crate) struct Cue<D> {
    kind: CueKind,
    detail: Option<D>,
    /// Part of the ending of a fight: kept, shortened, when the display falls behind.
    outcome: bool,
    /// Never skipped by the catch-up rule: a death, a critical hit, the end of the fight.
    keep: bool,
    short: bool,
    queued_ms: u64,
    /// The server's beat sequence this cue belongs to (`CombatOutcome.sequence`); 0 is no beat.
    sequence: u64,
}

/// One presented (or coherently skipped) phase of a beat, for the display's debug record:
/// checks assert sync from these numbers instead of sleeping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BeatRecord {
    pub(crate) sequence: u64,
    pub(crate) kind: &'static str,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
}

/// How many beat records the debug log keeps.
const BEAT_LOG_CAP: usize = 40;

/// What the browser layer applies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command<D> {
    Render,
    /// A state began; `detail` is the payload of the cue that began it.
    Enter {
        state: StageState,
        detail: Option<D>,
        short: bool,
    },
    /// The queue is empty and the last state ended.
    Rest,
}

/// The cues one accepted view contributes. Payloads are opaque to the director.
#[derive(Clone, Debug)]
pub(crate) struct ViewDiff<D> {
    pub(crate) scene_changed: bool,
    /// The scene change is the start of a fight: it is not held behind the line being spoken.
    pub(crate) enters_combat: bool,
    /// The story begins with this view: the scene change shows the title card (the payload)
    /// after the wordmark and before the scene enters.
    pub(crate) title: Option<D>,
    /// The server's active combatant changed in this view.
    pub(crate) turn_changed: bool,
    pub(crate) roll: Option<D>,
    pub(crate) impact: Option<D>,
    /// The target of the roll dropped to zero hit points.
    pub(crate) defeat: Option<D>,
    pub(crate) heal: Option<D>,
    /// The fight ended in this view; the payload describes the banner.
    pub(crate) banner: Option<D>,
    /// The closing card that follows the banner of a fight that ended in this view.
    pub(crate) closing: Option<D>,
    /// Only the blow of an earlier turn that the same view resolved: dice, impact and fall, with
    /// no redraw of the view, which belongs to the newest turn.
    pub(crate) blow_only: bool,
    /// This blow decided something the table must see even when behind: a critical hit or a
    /// fall. Deaths and fight endings are always kept without this flag.
    pub(crate) memorable: bool,
    /// The server sequence of the blow this diff stages (`CombatOutcome.sequence`); 0 when the
    /// view stages no beat. Beats present in this order and never twice: a diff at or below the
    /// presented sequence has its blow cues dropped (see [`StageDirector::gate`]).
    pub(crate) sequence: u64,
}

impl<D> ViewDiff<D> {
    /// What must stay immediate: dice, a defeat, a fight's start or end. It is never held
    /// behind story speech.
    pub(crate) fn is_outcome(&self) -> bool {
        self.enters_combat
            || self.roll.is_some()
            || self.impact.is_some()
            || self.defeat.is_some()
            || self.banner.is_some()
            || self.closing.is_some()
    }
}

impl<D> Default for ViewDiff<D> {
    fn default() -> Self {
        Self {
            scene_changed: false,
            enters_combat: false,
            title: None,
            turn_changed: false,
            roll: None,
            impact: None,
            defeat: None,
            heal: None,
            banner: None,
            closing: None,
            blow_only: false,
            memorable: false,
            sequence: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct Active<D> {
    state: StageState,
    detail: Option<D>,
    started_ms: u64,
    minimum_ms: u32,
    maximum_ms: u32,
    short: bool,
    sequence: u64,
}

pub(crate) struct StageDirector<D> {
    queue: VecDeque<Cue<D>>,
    active: Option<Active<D>>,
    /// The 3D battlefield is still playing the beats of an earlier blow.
    battlefield_busy: bool,
    /// Since when the next blow has been waiting for the battlefield.
    waiting_since_ms: Option<u64>,
    /// The last beat sequence finished presenting (or coherently skipped past). Only staged
    /// sequences count: server beats the director never cues (a move walked inside a blow, a
    /// death save) are covered by the next staged beat past them, or by true idle.
    presented: u64,
    /// The newest beat sequence any accepted view has carried.
    latest: u64,
    /// Every staged sequence of this fight, and the ones finished or skipped: `presented` is
    /// the highest staged sequence with no unresolved staged sequence at or below it.
    staged: std::collections::BTreeSet<u64>,
    resolved: std::collections::BTreeSet<u64>,
    beat_log: VecDeque<BeatRecord>,
}

fn phase_kind(state: StageState) -> &'static str {
    match state {
        StageState::Rolling => "rolling",
        StageState::Impact => "impact",
        StageState::Defeat => "defeat",
        StageState::Healing => "healing",
        StageState::Aftermath => "aftermath",
        StageState::ClosingCard => "closing",
        StageState::LeavingScene => "leaving",
        StageState::Interstitial => "interstitial",
        StageState::TitleCard => "title",
        StageState::EnteringScene => "entering",
        StageState::Idle => "idle",
    }
}

/// The longest a blow waits for the battlefield to finish the one before it.
const BATTLEFIELD_WAIT_CAP_MS: u64 = 20_000;

impl<D: Clone> Default for StageDirector<D> {
    fn default() -> Self {
        Self::new()
    }
}

fn shorten(milliseconds: u32, short: bool, divisor: u32) -> u32 {
    if short {
        (milliseconds / divisor).max(SHORT_FLOOR_MS)
    } else {
        milliseconds
    }
}

impl<D: Clone> StageDirector<D> {
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            active: None,
            battlefield_busy: false,
            waiting_since_ms: None,
            presented: 0,
            latest: 0,
            staged: std::collections::BTreeSet::new(),
            resolved: std::collections::BTreeSet::new(),
            beat_log: VecDeque::new(),
        }
    }

    /// The last beat sequence finished presenting (or coherently skipped past).
    pub(crate) fn presented_sequence(&self) -> u64 {
        self.presented
    }

    /// The recent beat phases with their start and end times, oldest first.
    pub(crate) fn beat_log(&self) -> &VecDeque<BeatRecord> {
        &self.beat_log
    }

    /// Nothing playing and nothing queued. `state()` alone reads Idle while a Roll waits for
    /// the battlefield, which must not count as done (the fight's exit once fired then).
    pub(crate) fn is_idle(&self) -> bool {
        self.active.is_none() && self.queue.is_empty()
    }

    fn record(&mut self, record: BeatRecord) {
        if self.beat_log.len() >= BEAT_LOG_CAP {
            self.beat_log.pop_front();
        }
        self.beat_log.push_back(record);
    }

    /// Marks a staged sequence finished or skipped and recomputes `presented`: the highest
    /// staged sequence with no unresolved staged sequence at or below it. A skipped middle
    /// blow therefore never reports presented ahead of a blow still playing before it.
    fn resolve(&mut self, sequence: u64) {
        if sequence == 0 {
            return;
        }
        self.resolved.insert(sequence);
        let mut presented = self.presented;
        for staged in &self.staged {
            if *staged <= presented {
                continue;
            }
            if self.resolved.contains(staged) {
                presented = *staged;
            } else {
                break;
            }
        }
        self.presented = presented;
    }

    fn reset_sequences(&mut self) {
        self.presented = 0;
        self.latest = 0;
        self.staged.clear();
        self.resolved.clear();
        self.beat_log.clear();
    }

    /// The one-clock gate for a newly accepted view's blow: a new fight restarts the sequence;
    /// a beat at or below the presented sequence (a duplicate view, a reconnect replay) loses
    /// its blow cues so nothing is ever shown twice, while the rest of the view (its scene
    /// change, its banner) still applies.
    fn gate(&mut self, diff: &mut ViewDiff<D>) {
        if diff.enters_combat {
            self.reset_sequences();
        }
        if diff.sequence == 0 {
            return;
        }
        self.latest = self.latest.max(diff.sequence);
        if diff.sequence <= self.presented || self.resolved.contains(&diff.sequence) {
            diff.roll = None;
            diff.impact = None;
            diff.defeat = None;
            diff.sequence = 0;
            return;
        }
        self.staged.insert(diff.sequence);
    }

    /// Whether the battlefield is still showing an earlier blow. A blow's dice wait for it, so
    /// each attack is seen in full before the next creature's begins.
    pub(crate) fn set_battlefield_busy(&mut self, busy: bool) {
        self.battlefield_busy = busy;
    }

    pub(crate) fn state(&self) -> StageState {
        self.active
            .as_ref()
            .map_or(StageState::Idle, |active| active.state)
    }

    #[cfg(test)]
    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Starts again at the current view with nothing queued: first view, reload, reconnect.
    /// Queued beats are skipped coherently, so they count as presented: the redraw that
    /// follows shows the newest state whole, on every layer at once.
    pub(crate) fn restart(&mut self) -> Vec<Command<D>> {
        let skipped: Vec<u64> = self
            .queue
            .iter()
            .map(|cue| cue.sequence)
            .chain(self.active.iter().map(|active| active.sequence))
            .filter(|sequence| *sequence != 0)
            .collect();
        self.queue.clear();
        self.active = None;
        self.waiting_since_ms = None;
        for sequence in skipped {
            self.resolve(sequence);
        }
        vec![Command::Render]
    }

    fn cue(kind: CueKind, detail: Option<D>, now_ms: u64, outcome: bool, sequence: u64) -> Cue<D> {
        Cue {
            kind,
            detail,
            outcome,
            keep: false,
            short: false,
            queued_ms: now_ms,
            sequence,
        }
    }

    /// Marks what the catch-up rule may never skip: deaths, fight endings, and the blows the
    /// caller flagged as memorable (a critical hit, a fall). Everything else may be shortened
    /// first, then skipped, oldest first, when the display falls behind.
    fn mark_unskippable(cues: &mut [Cue<D>], memorable: bool) {
        for cue in cues.iter_mut() {
            cue.keep = matches!(
                cue.kind,
                CueKind::Defeat | CueKind::Banner | CueKind::Closing
            ) || (memorable && matches!(cue.kind, CueKind::Roll | CueKind::Impact));
        }
    }

    /// Queues the cues of a newly accepted view and returns what starts now.
    pub(crate) fn on_view(&mut self, now_ms: u64, mut diff: ViewDiff<D>) -> Vec<Command<D>> {
        if diff.blow_only {
            return self.on_blow(now_ms, diff);
        }
        self.gate(&mut diff);
        let sequence = diff.sequence;
        let finishing = diff.banner.is_some();
        let memorable = diff.memorable;
        let make = |kind, detail| Self::cue(kind, detail, now_ms, finishing, sequence);
        let mut cues = Vec::new();
        if finishing || diff.scene_changed {
            // The results of the last blow come before the scene changes behind them.
            cues.extend(diff.roll.map(|d| make(CueKind::Roll, Some(d))));
            cues.extend(diff.impact.map(|d| make(CueKind::Impact, Some(d))));
            cues.extend(diff.defeat.map(|d| make(CueKind::Defeat, Some(d))));
            cues.push(Self::cue(
                CueKind::Scene,
                diff.title.clone(),
                now_ms,
                finishing,
                0,
            ));
            cues.extend(
                diff.banner
                    .map(|d| Self::cue(CueKind::Banner, Some(d), now_ms, finishing, 0)),
            );
            cues.extend(
                diff.closing
                    .map(|d| Self::cue(CueKind::Closing, Some(d), now_ms, finishing, 0)),
            );
        } else {
            // One clock on the flat stage too: a view carrying a blow is redrawn only after
            // the blow's impact, so no layer shows the result before the beat that causes it.
            // A view with no blow redraws at once, as before.
            let has_blow = diff.roll.is_some() || diff.impact.is_some() || diff.defeat.is_some();
            if !has_blow {
                cues.push(Self::cue(CueKind::Render, None, now_ms, false, 0));
            }
            cues.extend(
                diff.heal
                    .map(|d| Self::cue(CueKind::Heal, Some(d), now_ms, false, sequence)),
            );
            // A blow is an outcome: when the display is behind it is shortened, never dropped.
            cues.extend(
                diff.roll
                    .map(|d| Self::cue(CueKind::Roll, Some(d), now_ms, true, sequence)),
            );
            cues.extend(
                diff.impact
                    .map(|d| Self::cue(CueKind::Impact, Some(d), now_ms, true, sequence)),
            );
            cues.extend(
                diff.defeat
                    .map(|d| Self::cue(CueKind::Defeat, Some(d), now_ms, true, sequence)),
            );
            if has_blow {
                cues.push(Self::cue(CueKind::Render, None, now_ms, false, 0));
                // Redraws coalesce: only the newest view is ever drawn, so an earlier queued
                // redraw behind older blows would draw this same view too early (and inflate
                // the queue past the catch-up cap, skipping blows that must play).
                self.queue.retain(|cue| cue.kind != CueKind::Render);
            }
        }
        Self::mark_unskippable(&mut cues, memorable);
        self.queue.extend(cues);
        self.collapse_if_behind(now_ms);
        self.advance(now_ms)
    }

    /// Queues only the blow of a finishing view (dice, impact, fall): no redraw and no scene
    /// change. The scene change follows when the conductor says the blow has been seen out.
    pub(crate) fn on_blow(&mut self, now_ms: u64, mut diff: ViewDiff<D>) -> Vec<Command<D>> {
        self.gate(&mut diff);
        let sequence = diff.sequence;
        let make = |kind, detail| Self::cue(kind, Some(detail), now_ms, true, sequence);
        let memorable = diff.memorable;
        let mut cues = Vec::new();
        cues.extend(diff.roll.map(|d| make(CueKind::Roll, d)));
        cues.extend(diff.impact.map(|d| make(CueKind::Impact, d)));
        cues.extend(diff.defeat.map(|d| make(CueKind::Defeat, d)));
        Self::mark_unskippable(&mut cues, memorable);
        self.queue.extend(cues);
        self.collapse_if_behind(now_ms);
        self.advance(now_ms)
    }

    pub(crate) fn on_tick(&mut self, now_ms: u64) -> Vec<Command<D>> {
        self.collapse_if_behind(now_ms);
        self.advance(now_ms)
    }

    /// Drops stale cosmetic cues when the queue is long or the display is far behind. The
    /// outcome-bearing cues stay, shortened; the newest view is still drawn by the last
    /// `Render`/`Scene`.
    fn collapse_if_behind(&mut self, now_ms: u64) {
        let scenes_waiting = self
            .queue
            .iter()
            .filter(|cue| cue.kind == CueKind::Scene)
            .count();
        if scenes_waiting >= 2 {
            // Several scene changes converge to the newest: what the older views would have
            // narrated or said is skipped, while every outcome they carried is kept.
            let newest = self
                .queue
                .iter()
                .rposition(|cue| cue.kind == CueKind::Scene)
                .unwrap_or(0);
            let mut kept = VecDeque::new();
            for (position, cue) in self.queue.drain(..).enumerate() {
                let outcome = cue.outcome
                    || matches!(
                        cue.kind,
                        CueKind::Roll
                            | CueKind::Impact
                            | CueKind::Defeat
                            | CueKind::Banner
                            | CueKind::Closing
                    );
                if position >= newest || outcome {
                    kept.push_back(cue);
                }
            }
            self.queue = kept;
        }
        let behind = self
            .queue
            .front()
            .is_some_and(|oldest| now_ms.saturating_sub(oldest.queued_ms) > LAG_LIMIT_MS);
        if self.queue.len() <= QUEUE_CAP && !behind {
            return;
        }
        let mut kept: Vec<Cue<D>> = Vec::new();
        let mut dropped_render = false;
        for mut cue in self.queue.drain(..) {
            if cue.outcome || cue.kind == CueKind::Scene {
                cue.short = true;
                // Two scene changes converge to one: the newest view is drawn once.
                if cue.kind == CueKind::Scene
                    && let Some(position) = kept
                        .iter()
                        .rposition(|earlier| earlier.kind == CueKind::Scene)
                    && kept
                        .get(position + 1..)
                        .is_some_and(|after| after.iter().all(|later| later.kind == CueKind::Scene))
                {
                    kept.remove(position);
                }
                kept.push(cue);
            } else if cue.kind == CueKind::Render || cue.kind == CueKind::Scene {
                dropped_render = true;
            }
        }
        if dropped_render && !kept.iter().any(|cue| cue.kind == CueKind::Scene) {
            kept.push(Self::cue(CueKind::Render, None, now_ms, false, 0));
        }
        // Catch-up rule: the display may lag the server by at most LAG_LIMIT_MS (or QUEUE_CAP
        // queued cues). Past that, every kept cue plays shortened, and skippable middle blows
        // are dropped oldest-first so the queue converges on the newest server event instead of
        // replaying an unbounded backlog. Never skipped: a death (Defeat), a critical hit or
        // fall the caller flagged memorable, the end of the fight (Banner, Closing), and the
        // newest blow, which is what the turn label, roll card, health bars and acting figure
        // converge on.
        let before: std::collections::BTreeSet<u64> = kept
            .iter()
            .map(|cue| cue.sequence)
            .filter(|sequence| *sequence != 0)
            .collect();
        Self::skip_middle_blows(&mut kept);
        // A skipped blow is jumped past coherently: it counts as presented once every earlier
        // staged beat resolves, and the debug log says it was skipped, never replayed.
        let after: std::collections::BTreeSet<u64> = kept
            .iter()
            .map(|cue| cue.sequence)
            .filter(|sequence| *sequence != 0)
            .collect();
        for sequence in before.difference(&after) {
            self.record(BeatRecord {
                sequence: *sequence,
                kind: "skipped",
                start_ms: now_ms,
                end_ms: now_ms,
            });
            self.resolve(*sequence);
        }
        self.queue = kept.into();
    }

    /// Drops every queued blow but the newest unless one of its cues is unskippable. Blows are
    /// consecutive Roll/Impact/Defeat runs; a new blow starts at each Roll that follows an
    /// Impact or Defeat, and any other cue ends a run.
    fn skip_middle_blows(kept: &mut Vec<Cue<D>>) {
        let mut blows: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for (index, cue) in kept.iter().enumerate() {
            if !matches!(cue.kind, CueKind::Roll | CueKind::Impact | CueKind::Defeat) {
                if !current.is_empty() {
                    blows.push(std::mem::take(&mut current));
                }
                continue;
            }
            let starts_new = matches!(cue.kind, CueKind::Roll)
                && current
                    .iter()
                    .any(|&at| matches!(kept[at].kind, CueKind::Impact | CueKind::Defeat));
            if starts_new {
                blows.push(std::mem::take(&mut current));
            }
            current.push(index);
        }
        if !current.is_empty() {
            blows.push(current);
        }
        if blows.len() < 2 {
            return;
        }
        let last = blows.len() - 1;
        let drop: std::collections::HashSet<usize> = blows
            .iter()
            .enumerate()
            .filter(|(position, blow)| {
                // The head of the queue without a Roll is the on-screen remainder of the blow
                // already playing: it finishes instead of being cut mid-beat.
                let head_remainder = blow
                    .first()
                    .is_some_and(|&at| !matches!(kept[at].kind, CueKind::Roll));
                *position != last && !head_remainder && !blow.iter().any(|&at| kept[at].keep)
            })
            .flat_map(|(_, blow)| blow.iter().copied())
            .collect();
        if drop.is_empty() {
            return;
        }
        let mut remaining = Vec::with_capacity(kept.len());
        for (index, cue) in kept.drain(..).enumerate() {
            if !drop.contains(&index) {
                remaining.push(cue);
            }
        }
        *kept = remaining;
    }

    fn begin(&mut self, now_ms: u64, cue: &Cue<D>, state: StageState) -> (Active<D>, Command<D>) {
        let (minimum_ms, maximum_ms) = match state {
            StageState::Rolling => (shorten(1_150, cue.short, 3), shorten(1_150, cue.short, 3)),
            StageState::Impact => (shorten(650, cue.short, 3), shorten(650, cue.short, 3)),
            StageState::Defeat => (
                shorten(DEFEAT_HOLD_MS, cue.short, 3),
                shorten(DEFEAT_HOLD_MS, cue.short, 3),
            ),
            StageState::Healing => (900, 900),
            StageState::LeavingScene => (shorten(350, cue.short, 2), shorten(350, cue.short, 2)),
            StageState::Interstitial => (shorten(700, cue.short, 2), shorten(700, cue.short, 2)),
            StageState::EnteringScene => (shorten(800, cue.short, 2), shorten(800, cue.short, 2)),
            StageState::Aftermath => (shorten(1_200, cue.short, 3), shorten(1_200, cue.short, 3)),
            StageState::TitleCard => (
                shorten(TITLE_CARD_MS, cue.short, 3),
                shorten(TITLE_CARD_MS, cue.short, 3),
            ),
            StageState::ClosingCard => (
                shorten(CLOSING_CARD_MS, cue.short, 3),
                shorten(CLOSING_CARD_MS, cue.short, 3),
            ),
            StageState::Idle => (0, 0),
        };
        let active = Active {
            state,
            detail: cue.detail.clone(),
            started_ms: now_ms,
            minimum_ms,
            maximum_ms,
            short: cue.short,
            sequence: cue.sequence,
        };
        let command = Command::Enter {
            state,
            detail: cue.detail.clone(),
            short: cue.short,
        };
        (active, command)
    }

    fn finished(active: &Active<D>, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(active.started_ms);
        elapsed >= u64::from(active.maximum_ms).max(u64::from(active.minimum_ms))
    }

    fn advance(&mut self, now_ms: u64) -> Vec<Command<D>> {
        let mut commands = Vec::new();
        let mut ended_any = false;
        loop {
            if let Some(active) = self.active.as_ref() {
                if !Self::finished(active, now_ms) {
                    break;
                }
                let Some(ended) = self.active.take() else {
                    break;
                };
                // A late tick must not stretch the sequence: the next state starts when the
                // previous one was due to end.
                let ended_at = (ended.started_ms + u64::from(ended.maximum_ms)).min(now_ms);
                if ended.sequence != 0 {
                    self.record(BeatRecord {
                        sequence: ended.sequence,
                        kind: phase_kind(ended.state),
                        start_ms: ended.started_ms,
                        end_ms: ended_at,
                    });
                    // The beat is presented when its last queued phase ends.
                    if !self.queue.iter().any(|cue| cue.sequence == ended.sequence) {
                        self.resolve(ended.sequence);
                    }
                }
                if let Some(next) = self.next_phase(&ended) {
                    self.start_phase(ended_at, next, &mut commands);
                    continue;
                }
                ended_any = true;
            }
            let waiting_for_battlefield = self.battlefield_busy
                && self
                    .queue
                    .front()
                    .is_some_and(|front| front.kind == CueKind::Roll)
                && now_ms.saturating_sub(*self.waiting_since_ms.get_or_insert(now_ms))
                    < BATTLEFIELD_WAIT_CAP_MS;
            if waiting_for_battlefield {
                if ended_any {
                    commands.push(Command::Rest);
                }
                break;
            }
            self.waiting_since_ms = None;
            let Some(cue) = self.queue.pop_front() else {
                if ended_any {
                    commands.push(Command::Rest);
                }
                break;
            };
            let state = match cue.kind {
                CueKind::Render => {
                    commands.push(Command::Render);
                    continue;
                }
                CueKind::Scene => StageState::LeavingScene,
                CueKind::Roll => StageState::Rolling,
                CueKind::Impact => StageState::Impact,
                CueKind::Defeat => StageState::Defeat,
                CueKind::Heal => StageState::Healing,
                CueKind::Banner => StageState::Aftermath,
                CueKind::Closing => StageState::ClosingCard,
            };
            self.start_phase(now_ms, (cue, state), &mut commands);
        }
        commands
    }

    /// The state that follows a scene change's leaving and interstitial phases.
    fn next_phase(&self, ended: &Active<D>) -> Option<(Cue<D>, StageState)> {
        // A scene change that carries a payload is the story's beginning: its title card
        // shows between the wordmark and the scene entering.
        let next = match ended.state {
            StageState::LeavingScene => StageState::Interstitial,
            StageState::Interstitial if ended.detail.is_some() => StageState::TitleCard,
            StageState::Interstitial | StageState::TitleCard => StageState::EnteringScene,
            _ => return None,
        };
        Some((
            Cue {
                kind: CueKind::Scene,
                detail: ended.detail.clone(),
                outcome: false,
                keep: false,
                short: ended.short,
                queued_ms: ended.started_ms,
                sequence: ended.sequence,
            },
            next,
        ))
    }

    fn start_phase(
        &mut self,
        now_ms: u64,
        (cue, state): (Cue<D>, StageState),
        out: &mut Vec<Command<D>>,
    ) {
        if state == StageState::Interstitial {
            // The painting changes while the wordmark covers it.
            out.push(Command::Render);
        }
        let (active, command) = self.begin(now_ms, &cue, state);
        self.active = Some(active);
        out.push(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Director = StageDirector<&'static str>;

    fn states(commands: &[Command<&'static str>]) -> Vec<String> {
        commands
            .iter()
            .map(|command| match command {
                Command::Render => "render".to_owned(),
                Command::Enter { state, .. } => format!("{state:?}"),
                Command::Rest => "rest".to_owned(),
            })
            .collect()
    }

    /// Ticks in 50 ms steps and gathers every command until `until_ms`.
    fn run(director: &mut Director, from_ms: u64, until_ms: u64) -> Vec<Command<&'static str>> {
        let mut all = Vec::new();
        let mut now = from_ms;
        while now <= until_ms {
            all.extend(director.on_tick(now));
            now += 50;
        }
        all
    }

    fn killing_blow() -> ViewDiff<&'static str> {
        ViewDiff {
            scene_changed: true,
            roll: Some("roll"),
            impact: Some("impact"),
            defeat: Some("defeat"),
            banner: Some("victory"),
            ..ViewDiff::default()
        }
    }

    fn blow(label: &'static str) -> ViewDiff<&'static str> {
        ViewDiff {
            roll: Some(label),
            impact: Some(label),
            blow_only: true,
            ..ViewDiff::default()
        }
    }

    #[test]
    fn a_blow_waits_for_the_battlefield_to_finish_the_one_before_it() {
        let mut director = Director::new();
        director.set_battlefield_busy(false);
        let mut commands = director.on_view(0, blow("first"));
        commands.extend(director.on_view(0, blow("second")));
        // The battlefield is busy with the first blow's beats from the first roll on.
        director.set_battlefield_busy(true);
        commands.extend(run(&mut director, 50, 6_000));
        let rolls = |commands: &[Command<&'static str>]| {
            commands
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        Command::Enter {
                            state: StageState::Rolling,
                            ..
                        }
                    )
                })
                .count()
        };
        assert_eq!(rolls(&commands), 1, "the second blow is held back");
        director.set_battlefield_busy(false);
        let later = run(&mut director, 6_050, 9_000);
        assert_eq!(rolls(&later), 1, "and begins once the battlefield is done");
        // A battlefield that never finishes cannot hold the table for ever.
        let mut stuck = Director::new();
        stuck.on_view(0, blow("first"));
        stuck.on_view(0, blow("second"));
        stuck.set_battlefield_busy(true);
        let all = run(&mut stuck, 50, 40_000);
        assert_eq!(rolls(&all), 1);
    }

    #[test]
    fn a_killing_blow_plays_before_the_scene_changes() {
        let mut director = Director::new();
        let mut commands = director.on_view(0, killing_blow());
        commands.extend(run(&mut director, 50, 12_000));
        let order: Vec<String> = states(&commands)
            .into_iter()
            .filter(|name| name != "rest")
            .collect();
        assert_eq!(
            order,
            [
                "Rolling",
                "Impact",
                "Defeat",
                "LeavingScene",
                "render",
                "Interstitial",
                "EnteringScene",
                "Aftermath"
            ]
        );
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn two_views_ten_milliseconds_apart_lose_no_outcome_cue() {
        let mut director = Director::new();
        let mut commands = director.on_view(0, killing_blow());
        commands.extend(director.on_view(
            10,
            ViewDiff {
                scene_changed: true,
                ..ViewDiff::default()
            },
        ));
        commands.extend(run(&mut director, 60, 20_000));
        let order = states(&commands);
        // The sequence outlasts the lag limit, but the outcome cues all play, in order.
        let positions: Vec<usize> = ["Rolling", "Impact", "Defeat", "Aftermath"]
            .iter()
            .map(|expected| {
                order
                    .iter()
                    .position(|name| name == expected)
                    .unwrap_or_else(|| panic!("{expected} in {order:?}"))
            })
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn backlog_collapse_keeps_outcome_cues_in_shortened_form() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                heal: Some("blocker"),
                ..ViewDiff::default()
            },
        );
        for at in 1..=12 {
            director.on_view(
                at,
                ViewDiff {
                    roll: Some("cosmetic roll"),
                    heal: Some("cosmetic heal"),
                    ..ViewDiff::default()
                },
            );
        }
        // Three fights ending in quick succession leave more than the cap waiting.
        let mut commands = director.on_view(20, killing_blow());
        commands.extend(director.on_view(21, killing_blow()));
        commands.extend(director.on_view(22, killing_blow()));
        commands.extend(run(&mut director, 70, 30_000));
        let order = states(&commands);
        assert!(!order.iter().any(|name| name == "Healing"), "{order:?}");
        for expected in ["Rolling", "Impact", "Defeat", "Aftermath"] {
            assert!(
                order.iter().any(|name| name == expected),
                "{expected} in {order:?}"
            );
        }
        let shortened = commands.iter().any(|command| {
            matches!(
                command,
                Command::Enter {
                    state: StageState::Rolling,
                    short: true,
                    ..
                }
            )
        });
        assert!(shortened);
    }

    #[test]
    fn a_display_far_behind_collapses_on_tick() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                defeat: Some("blocker"),
                ..ViewDiff::default()
            },
        );
        director.on_view(
            1,
            ViewDiff {
                heal: Some("stale"),
                ..ViewDiff::default()
            },
        );
        let commands = director.on_tick(LAG_LIMIT_MS + 100);
        assert!(!states(&commands).iter().any(|name| name == "Healing"));
        assert!(director.queue_len() <= 1);
    }

    #[test]
    fn a_restart_begins_at_the_current_view_with_an_empty_queue() {
        let mut director = Director::new();
        director.on_view(0, killing_blow());
        assert_ne!(director.queue_len(), 0);
        let commands = director.restart();
        assert_eq!(commands, [Command::Render]);
        assert_eq!(director.queue_len(), 0);
        assert_eq!(director.state(), StageState::Idle);
        assert!(director.on_tick(50).is_empty());
    }

    fn names(commands: &[Command<&'static str>]) -> Vec<String> {
        states(commands)
            .into_iter()
            .filter(|name| name != "rest")
            .collect()
    }

    #[test]
    fn the_story_begins_with_the_wordmark_then_the_title_card_then_the_scene() {
        let mut director = Director::new();
        let mut commands = director.on_view(
            0,
            ViewDiff {
                scene_changed: true,
                title: Some("title"),
                ..ViewDiff::default()
            },
        );
        commands.extend(run(&mut director, 50, 9_000));
        assert_eq!(
            names(&commands),
            [
                "LeavingScene",
                "render",
                "Interstitial",
                "TitleCard",
                "EnteringScene"
            ]
        );
        let card = commands.iter().find_map(|command| match command {
            Command::Enter {
                state: StageState::TitleCard,
                detail,
                ..
            } => *detail,
            _ => None,
        });
        assert_eq!(
            card,
            Some("title"),
            "the card's payload reaches the browser"
        );
    }

    #[test]
    fn the_title_card_lasts_about_three_seconds() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                scene_changed: true,
                title: Some("title"),
                ..ViewDiff::default()
            },
        );
        // Leaving 350 ms and the interstitial 700 ms come first.
        director.on_tick(1_050);
        assert_eq!(director.state(), StageState::TitleCard);
        director.on_tick(1_050 + 2_950);
        assert_eq!(director.state(), StageState::TitleCard);
        director.on_tick(1_050 + 3_000);
        assert_eq!(director.state(), StageState::EnteringScene);
    }

    #[test]
    fn an_ordinary_scene_change_has_no_title_card() {
        let mut director = Director::new();
        let mut commands = director.on_view(
            0,
            ViewDiff {
                scene_changed: true,
                ..ViewDiff::default()
            },
        );
        commands.extend(run(&mut director, 50, 4_000));
        assert!(!names(&commands).iter().any(|name| name == "TitleCard"));
    }

    #[test]
    fn a_won_or_lost_fight_ends_with_the_closing_card_after_the_banner() {
        let mut director = Director::new();
        let mut diff = killing_blow();
        diff.closing = Some("closing");
        let mut commands = director.on_view(0, diff);
        commands.extend(run(&mut director, 50, 16_000));
        let order = names(&commands);
        assert_eq!(
            &order[order.len() - 3..],
            ["EnteringScene", "Aftermath", "ClosingCard"]
        );
        assert_eq!(director.state(), StageState::Idle);
    }

    /// A blow the caller flagged memorable: a critical hit with no fall.
    fn hard_hit(label: &'static str) -> ViewDiff<&'static str> {
        ViewDiff {
            roll: Some(label),
            impact: Some(label),
            blow_only: true,
            memorable: true,
            ..ViewDiff::default()
        }
    }

    fn rolling_labels(commands: &[Command<&'static str>]) -> Vec<&'static str> {
        commands
            .iter()
            .filter_map(|command| match command {
                Command::Enter {
                    state: StageState::Rolling,
                    detail: Some(label),
                    ..
                } => Some(*label),
                _ => None,
            })
            .collect()
    }

    fn enters(commands: &[Command<&'static str>], state: StageState) -> usize {
        commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::Enter {
                        state: entered,
                        ..
                    } if *entered == state
                )
            })
            .count()
    }

    #[test]
    fn a_burst_of_enemy_outcomes_faster_than_staging_skips_middle_blows() {
        let mut director = Director::new();
        let labels = ["b0", "b1", "b2", "b3", "b4", "b5", "b6", "b7"];
        let mut commands = Vec::new();
        for (n, label) in labels.iter().enumerate() {
            commands.extend(director.on_view((n as u64) * 100, blow(label)));
        }
        commands.extend(run(&mut director, 800, 12_000));
        let staged = rolling_labels(&commands);
        // The first blow was already on screen and the newest is what the table converges on.
        assert_eq!(staged.first(), Some(&"b0"));
        assert_eq!(staged.last(), Some(&"b7"));
        // The backlog never replays in full: at most a few blows are staged, and the early
        // middle blows are skipped.
        assert!(
            staged.len() <= 4,
            "bounded replay, not the whole burst: {staged:?}"
        );
        for skipped in ["b1", "b2", "b3", "b4"] {
            assert!(
                !staged.contains(&skipped),
                "obsolete middle blow {skipped} is skipped: {staged:?}"
            );
        }
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn a_turn_change_mid_beat_keeps_the_critical_hit_and_jumps_to_the_newest_turn() {
        let mut director = Director::new();
        let mut commands = director.on_view(0, blow("turn-a"));
        commands.extend(director.on_view(100, blow("turn-b")));
        commands.extend(director.on_view(200, hard_hit("turn-c")));
        commands.extend(director.on_view(300, blow("turn-d")));
        commands.extend(director.on_view(400, blow("turn-e")));
        commands.extend(director.on_view(500, blow("turn-f")));
        commands.extend(director.on_view(600, blow("turn-g")));
        commands.extend(director.on_view(700, blow("turn-h")));
        commands.extend(run(&mut director, 750, 14_000));
        let staged = rolling_labels(&commands);
        // The ordinary early turns are skipped, but the critical hit plays in order and the
        // newest turn is always staged, so the label lands on the server's turn.
        assert_eq!(staged.first(), Some(&"turn-a"));
        assert_eq!(staged.last(), Some(&"turn-h"));
        assert!(staged.contains(&"turn-c"), "critical hit kept: {staged:?}");
        for skipped in ["turn-b", "turn-d", "turn-e"] {
            assert!(
                !staged.contains(&skipped),
                "obsolete turn {skipped} skipped: {staged:?}"
            );
        }
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn a_fight_ending_while_beats_are_queued_plays_one_short_ending() {
        let mut director = Director::new();
        let labels = ["e0", "e1", "e2", "e3", "e4", "e5"];
        let mut commands = Vec::new();
        for (n, label) in labels.iter().enumerate() {
            commands.extend(director.on_view((n as u64) * 100, blow(label)));
        }
        // The server ends the fight while blows are still queued: the killing blow plays, then
        // exactly one banner and one closing card, in bounded time.
        let mut end = killing_blow();
        end.closing = Some("closing");
        commands.extend(director.on_view(600, end));
        commands.extend(run(&mut director, 650, 20_000));
        assert_eq!(enters(&commands, StageState::Aftermath), 1);
        assert_eq!(enters(&commands, StageState::ClosingCard), 1);
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn a_fallen_character_is_held_long_enough_for_the_table_to_see_the_blow() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                roll: Some("roll"),
                impact: Some("impact"),
                defeat: Some("defeat"),
                ..ViewDiff::default()
            },
        );
        // Rolling 1.15 s and the impact 0.65 s come first; the defeat then holds 2.5 s.
        run(&mut director, 50, 1_850);
        assert_eq!(director.state(), StageState::Defeat);
        run(&mut director, 1_900, 4_250);
        assert_eq!(director.state(), StageState::Defeat);
        run(&mut director, 4_300, 4_450);
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn catch_up_does_not_erase_a_pending_scene_change() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                defeat: Some("active blocker"),
                ..ViewDiff::default()
            },
        );
        director.on_view(
            1,
            ViewDiff {
                heal: Some("stale queued cue"),
                ..ViewDiff::default()
            },
        );

        let commands = director.on_view(
            4_002,
            ViewDiff {
                scene_changed: true,
                ..ViewDiff::default()
            },
        );

        assert!(
            states(&commands)
                .iter()
                .any(|state| state == "LeavingScene"),
            "the submitted scene change must survive catch-up"
        );
    }

    fn sequenced_blow(sequence: u64, label: &'static str) -> ViewDiff<&'static str> {
        ViewDiff {
            roll: Some(label),
            impact: Some(label),
            sequence,
            ..ViewDiff::default()
        }
    }

    #[test]
    fn presented_advances_only_when_the_beats_last_phase_ends() {
        let mut director = Director::new();
        director.on_view(0, sequenced_blow(3, "blow"));
        assert_eq!(director.latest, 3);
        assert_eq!(director.presented_sequence(), 0);
        // The roll (1.15 s) ending is not the beat presented; the impact is still to come.
        run(&mut director, 50, 1_200);
        assert_eq!(director.presented_sequence(), 0);
        run(&mut director, 1_250, 2_000);
        assert_eq!(director.presented_sequence(), 3);
        let kinds: Vec<&str> = director
            .beat_log()
            .iter()
            .map(|record| record.kind)
            .collect();
        assert_eq!(kinds, ["rolling", "impact"]);
        assert!(
            director
                .beat_log()
                .iter()
                .all(|record| record.sequence == 3)
        );
    }

    #[test]
    fn a_replayed_beat_is_never_staged_twice() {
        let mut director = Director::new();
        director.on_view(0, sequenced_blow(1, "first"));
        run(&mut director, 50, 2_000);
        assert_eq!(director.presented_sequence(), 1);
        // The same view again (a duplicate, a reconnect): no roll is staged again.
        let commands = director.on_view(2_100, sequenced_blow(1, "first"));
        assert!(!states(&commands).iter().any(|state| state == "Rolling"));
        run(&mut director, 2_150, 4_000);
        assert_eq!(director.presented_sequence(), 1);
        assert_eq!(director.beat_log().len(), 2);
    }

    #[test]
    fn a_new_fight_restarts_the_sequence() {
        let mut director = Director::new();
        director.on_view(0, sequenced_blow(9, "old"));
        run(&mut director, 50, 2_000);
        assert_eq!(director.presented_sequence(), 9);
        let mut opening = sequenced_blow(1, "new");
        opening.enters_combat = true;
        director.on_view(2_100, opening);
        assert_eq!(director.latest, 1);
        run(&mut director, 2_150, 4_500);
        assert_eq!(director.presented_sequence(), 1);
    }

    #[test]
    fn is_idle_is_false_while_a_roll_waits_for_the_battlefield() {
        let mut director = Director::new();
        director.set_battlefield_busy(true);
        director.on_view(0, sequenced_blow(1, "held"));
        // No state is active (the roll waits), yet the director is not done.
        assert_eq!(director.state(), StageState::Idle);
        assert!(!director.is_idle());
        director.set_battlefield_busy(false);
        run(&mut director, 50, 2_500);
        assert!(director.is_idle());
        assert_eq!(director.presented_sequence(), 1);
    }
}
