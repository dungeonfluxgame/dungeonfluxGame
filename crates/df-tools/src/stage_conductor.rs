//! The conductor of the shared display: it owns the scene, line, clip and actor machines of
//! `stage_machines` and the cue sequencer of `stage_director`, and turns view arrivals, voice
//! events and timer ticks into commands for the browser layer.
//!
//! The rules it enforces:
//! - A newly arrived view that changes the scene only sets the scene's pending successor. The
//!   swap happens in the scene's Exiting, after the lines of the scene on screen have been
//!   spoken. A view that merely adds to the scene (a typed line, a reply) is queued behind the
//!   lines already waiting, in order.
//! - Exactly one line is Opening, Speaking or Lingering at a time. A voiced line opens when its
//!   clip starts; a line without sound opens at its turn and is read for a length-based time,
//!   so pacing matches. A lost media event takes the normal exit when its bound passes.
//! - A newer view never cuts speech: the clip in progress finishes. When more than one scene
//!   change waits, remaining queued lines go to the log without a flash. The current line still
//!   finishes its voice and reading time before the display converges on the newest scene.
//! - Outcomes (dice, a defeat, a fight's start or end) are never held behind story speech: the
//!   line in progress fades within [`FIGHT_LINE_FADE_MS`] and the director plays the outcome.
//!
//! There is no clock inside: callers pass `now_ms`, so tests use a controlled one. The
//! conductor decides nothing about the game and sends nothing; the newest accepted view is
//! always what a `Render` command draws.
use crate::{
    stage_director::{Command as DirectorCommand, StageDirector, StageState, ViewDiff},
    stage_machines::{
        ACTOR_ENTER_MS, ACTOR_EXIT_MS, ActorEvent, ActorState, BATTLE_ENTER_CAP_MS,
        BATTLE_FINISH_CAP_MS, BattleEvent, BattleState, CLIP_END_GRACE_MS, CLIP_READY_WAIT_MS,
        CLIP_START_WAIT_MS, ClipEvent, ClipState, FIGHT_LINE_FADE_MS, FINAL_LINGER_CAP_MS,
        HELD_OUTCOMES_CAP, INTENT_EXPIRY_MS, LINE_CLOSE_MS, LINE_LINGER_MS, LINE_OPEN_MS,
        LineEvent, LineState, SCENE_ENTER_CAP_MS, SCENE_EXIT_CAP_MS, SCENE_WAIT_CAP_MS, SceneEvent,
        SceneState, actor_next, battle_next, clip_next, line_next, scene_next,
    },
};

pub(crate) type LineId = u64;

const BEAT_CLIP_WAIT_MS: u64 = 1_500;
const BEAT_SPEECH_CAP_MS: u64 = 10_000;

/// One line a view puts on the stage. The payload is opaque to the conductor.
#[derive(Clone, Debug)]
pub(crate) struct ScriptLine<D> {
    pub(crate) detail: D,
    /// A clip can be played for it (narration and non-player speech); a hero's typed line and
    /// an action call-out are silent.
    pub(crate) voiceable: bool,
    /// Minimum readable time, whether or not sound plays.
    pub(crate) read_ms: u32,
    /// The actor slot whose figure carries the bubble; `None` for the narrator's box.
    pub(crate) slot: Option<usize>,
    /// Already on screen before the page was (reload, reconnect): it stands Lingering,
    /// silently, with no entrance.
    pub(crate) settled: bool,
}

/// What a hero is doing in the dialogue box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntentKind {
    Typing,
    Speaking,
    Submitted,
}

/// One hero's live speech intent, from the view. The payload is opaque to the conductor.
#[derive(Clone, Debug)]
pub(crate) struct IntentMark<D> {
    /// The hero's actor slot.
    pub(crate) slot: usize,
    pub(crate) kind: IntentKind,
    pub(crate) detail: D,
}

/// What one accepted view contributes.
#[derive(Clone, Debug)]
pub(crate) struct ViewPlan<D> {
    /// The cues for the cue sequencer (no speech; lines are in `script`).
    pub(crate) diff: ViewDiff<D>,
    pub(crate) script: Vec<ScriptLine<D>>,
    /// Slots of the actors standing in the scene this view shows.
    pub(crate) cast: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipOp {
    /// Fetch and decode the clip, without playing it.
    Request,
    /// Play the requested clip now.
    Play,
    /// Stop it, or drop it unplayed.
    Cancel,
}

/// What the browser layer applies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command<D> {
    Render,
    Skip {
        detail: Option<D>,
    },
    /// A cue-sequencer state began (transition, dice, impact, banner, cards).
    Enter {
        state: StageState,
        detail: Option<D>,
        short: bool,
    },
    Rest,
    Scene(SceneState),
    Line {
        id: LineId,
        state: LineState,
        detail: D,
        slot: Option<usize>,
    },
    Clip {
        id: LineId,
        op: ClipOp,
        detail: D,
    },
    Actor {
        slot: usize,
        state: ActorState,
    },
    Battle(BattleState),
    /// The state of a line's voice clip changed; a clip that failed or was skipped is reported
    /// as such instead of looking spoken.
    ClipState {
        id: LineId,
        state: ClipState,
    },
}

/// How far the 3D battlefield is with a fight's entrance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Battlefield {
    /// Still mounting, or still playing its initiative roll and roll-call.
    Pending,
    /// Mounted and its entrance is done.
    Ready,
    /// It will not appear (no WebGPU, device lost): the flat combat view is the display.
    Fallback,
}

/// What the voice player reports about a clip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipSignal {
    Loaded,
    /// Playback began; the clip's length is known once the browser reads its header.
    Started {
        duration_ms: Option<u32>,
    },
    Finished,
    Error,
}

#[derive(Clone, Debug)]
struct LineRun<D> {
    id: LineId,
    line: ScriptLine<D>,
    state: LineState,
    clip: ClipState,
    /// When the line entered its state.
    since_ms: u64,
    /// When the clip entered its state.
    clip_since_ms: u64,
    /// The longest the playing clip may run before its missing end signal is assumed.
    clip_limit_ms: u32,
    play_sent: bool,
    /// Reading starts when the bubble opens, independently of the clip's end time.
    opened_ms: Option<u64>,
    close_ms: u32,
    /// The line began with its clip playing.
    voiced: bool,
    /// The clip state last reported to the display.
    reported_clip: ClipState,
    /// A speech-intent loader, not a line of text: it never takes a turn.
    intent: Option<IntentKind>,
    /// When the table last affirmed the loader (a view carrying it).
    affirmed_ms: u64,
    timing: df_observe::presentation::TimingEvent,
}

struct Pending<D> {
    diff: ViewDiff<D>,
    script: Vec<ScriptLine<D>>,
    cast: Vec<usize>,
    /// Scene changes merged into this successor: the table has moved on this many times.
    arrivals: u32,
    since_ms: u64,
    /// The director has been given the scene change.
    submitted: bool,
}

struct ActorRun {
    slot: usize,
    state: ActorState,
    since_ms: u64,
}

pub(crate) struct Conductor<D> {
    line_timings: std::collections::VecDeque<df_observe::presentation::TimingEvent>,
    scene: SceneState,
    scene_since_ms: u64,
    director: StageDirector<D>,
    lines: Vec<LineRun<D>>,
    pending: Option<Pending<D>>,
    actors: Vec<ActorRun>,
    next_id: LineId,
    sound_on: bool,
    /// The director is in its entrance state, which ends the scene's Entering.
    in_entrance: bool,
    battle: BattleState,
    battle_since_ms: u64,
    battle_progress_ms: u64,
    /// A 3D battlefield is expected, so the fight's entrance waits for it (and the flat view
    /// stays hidden meanwhile instead of flashing before it).
    battlefield_expected: bool,
    battlefield: Battlefield,
    /// Dice and blows that arrived during the entrance: shown only after it.
    held: Vec<ViewDiff<D>>,
    /// The fight's last blow is on its way: the banner is queued behind it.
    finishing: bool,
    /// The scene change that ends a fight, held until its finishing blow has been shown out:
    /// the display never takes the victory scene while the battlefield is still playing.
    held_exit: Option<ViewDiff<D>>,
    /// The turn changed while a beat was playing: the next combatant's turn opens when it ends.
    turn_pending: bool,
    /// When the blow's cues finished and the exit began waiting for the battlefield.
    exit_waiting_since_ms: Option<u64>,
    /// The current blow's speech may hold its successor, but never beyond ten seconds.
    beat_since_ms: Option<u64>,
}

impl<D: Clone> Default for Conductor<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Clone> Conductor<D> {
    pub(crate) fn new() -> Self {
        Self {
            line_timings: std::collections::VecDeque::new(),
            scene: SceneState::Hidden,
            scene_since_ms: 0,
            director: StageDirector::new(),
            lines: Vec::new(),
            pending: None,
            actors: Vec::new(),
            next_id: 1,
            sound_on: false,
            in_entrance: false,
            battle: BattleState::Absent,
            battle_since_ms: 0,
            battle_progress_ms: 0,
            battlefield_expected: false,
            battlefield: Battlefield::Pending,
            held: Vec::new(),
            finishing: false,
            held_exit: None,
            turn_pending: false,
            exit_waiting_since_ms: None,
            beat_since_ms: None,
        }
    }

    /// Whether the adapter should read the battlefield: a fight's entrance or exit waits on it.
    pub(crate) fn wants_battlefield(&self) -> bool {
        self.battle == BattleState::Entering || self.held_exit.is_some()
    }

    /// Whether the 3D battlefield is still showing an earlier blow: the next one waits for it.
    pub(crate) fn set_battlefield_busy(&mut self, busy: bool) {
        self.director.set_battlefield_busy(busy);
    }

    pub(crate) fn battle_state(&self) -> BattleState {
        self.battle
    }

    /// The fight owns the stage from its entrance through its exit unless the renderer failed.
    /// A pending renderer while no fight is present must never hide the scene cast.
    pub(crate) fn battlefield_live(&self) -> bool {
        self.battle != BattleState::Absent
            && self.battlefield_expected
            && self.battlefield != Battlefield::Fallback
    }

    /// The last beat sequence the cue sequencer finished presenting (one clock).
    pub(crate) fn presented_sequence(&self) -> u64 {
        self.director.presented_sequence()
    }

    pub(crate) fn observe_presented(&mut self, sequence: u64) {
        self.director.observe_presented(sequence);
    }

    /// The recent beat phases with their start and end times, for the debug record.
    pub(crate) fn beat_log(
        &self,
    ) -> &std::collections::VecDeque<crate::stage_director::BeatRecord> {
        self.director.beat_log()
    }

    /// Presentation-clock facts only. The DOM adapter supplies scene/character metadata and
    /// reports them only after successful application; this snapshot grants no game authority.
    pub(crate) fn line_timing(&self, id: LineId) -> Option<df_observe::presentation::TimingEvent> {
        self.line_timings
            .iter()
            .find(|timing| timing.id == id)
            .cloned()
    }

    fn retain_line_timing(&mut self, timing: df_observe::presentation::TimingEvent) {
        self.line_timings
            .retain(|previous| previous.id != timing.id);
        self.line_timings.push_back(timing);
        if self.line_timings.len() > 128 {
            self.line_timings.pop_front();
        }
    }

    /// The cue sequencer has nothing playing and nothing queued (a Roll waiting for the
    /// battlefield does not count as idle), and no beat is held back for the battlefield's
    /// entrance or exit: a held beat has not been presented, however empty the sequencer is.
    pub(crate) fn director_idle(&self) -> bool {
        self.director.is_idle()
            && self.battle != BattleState::Entering
            && self.held.is_empty()
            && self.held_exit.is_none()
    }

    /// Whether a 3D battlefield will show the next fight (WebGPU is present).
    pub(crate) fn set_battlefield_expected(&mut self, expected: bool) {
        self.battlefield_expected = expected;
    }

    /// The adapter's reading of the battlefield while a fight's entrance waits for it.
    pub(crate) fn on_battlefield(&mut self, now_ms: u64, state: Battlefield) -> Vec<Command<D>> {
        self.battlefield = state;
        let mut out = Vec::new();
        self.step(now_ms, &mut out);
        out
    }

    /// The battlefield's visible entrance advanced; restart its stalled-load deadline.
    pub(crate) fn on_battlefield_progress(&mut self, now_ms: u64) {
        if self.battle == BattleState::Entering {
            self.battle_progress_ms = now_ms;
        }
    }

    /// Whether a pending battlefield exceeded its progress deadline and took the flat fallback.
    pub(crate) fn battlefield_failed(&self) -> bool {
        self.battlefield == Battlefield::Fallback
    }

    pub(crate) fn scene_state(&self) -> SceneState {
        self.scene
    }

    #[cfg(test)]
    pub(crate) fn director_state(&self) -> StageState {
        self.director.state()
    }

    /// Whether a successor scene waits for the scene on screen.
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The state of the line the display is showing, with its id: the line being heard or
    /// read, else one fading out, else one waiting for its clip.
    pub(crate) fn focus_line(&self) -> Option<(LineId, LineState)> {
        let rank = |state: LineState| match state {
            LineState::Opening | LineState::Speaking | LineState::Lingering => 0,
            LineState::Closing => 1,
            LineState::Queued => 2,
            LineState::Composing | LineState::Submitted | LineState::Cancelled => 3,
            LineState::Done => 4,
        };
        self.lines
            .iter()
            .filter(|run| run.state != LineState::Done)
            .min_by_key(|run| rank(run.state))
            .map(|run| (run.id, run.state))
    }

    /// Whether a line is still to be shown or is being shown: the thinking mark waits for it.
    pub(crate) fn lines_waiting(&self) -> bool {
        self.lines.iter().any(|run| {
            matches!(
                run.state,
                LineState::Queued | LineState::Opening | LineState::Speaking
            )
        })
    }

    /// The slot of the figure whose line is waiting for its voice clip (queued, its clip on the
    /// way): the display keeps the thinking mark there until the clip starts.
    pub(crate) fn awaiting_clip_slot(&self) -> Option<usize> {
        self.lines
            .iter()
            .find(|run| {
                run.state == LineState::Queued
                    && matches!(run.clip, ClipState::Requested | ClipState::Ready)
            })
            .and_then(|run| run.line.slot)
    }

    /// Reports every clip whose state changed since the display last heard of it.
    fn report_clips(&mut self, out: &mut Vec<Command<D>>) {
        for run in &mut self.lines {
            if run.clip != run.reported_clip && run.line.voiceable {
                run.reported_clip = run.clip;
                out.push(Command::ClipState {
                    id: run.id,
                    state: run.clip,
                });
            }
        }
    }

    /// The slot of the figure the active line belongs to; `Some(None)` is the narrator's box.
    pub(crate) fn active_speaker(&self) -> Option<Option<usize>> {
        self.lines
            .iter()
            .find(|run| run.state.is_active())
            .map(|run| run.line.slot)
    }

    /// Whether anything still needs the clock: a transition, a waiting successor, a line
    /// being shown or one that will be, or an actor walking. A final line lingering with
    /// nothing behind it needs only an event.
    pub(crate) fn is_busy(&self) -> bool {
        let lines = self
            .lines
            .iter()
            .enumerate()
            .any(|(index, run)| match run.state {
                LineState::Done => false,
                LineState::Lingering => {
                    self.pending.is_some()
                        || self
                            .lines
                            .iter()
                            .skip(index + 1)
                            .any(|later| later.state == LineState::Queued)
                }
                _ => true,
            });
        !self.director.is_idle()
            || matches!(self.scene, SceneState::Entering | SceneState::Exiting)
            || self.pending.is_some()
            || self.battle == BattleState::Entering
            || self.held_exit.is_some()
            || lines
            || self
                .actors
                .iter()
                .any(|actor| matches!(actor.state, ActorState::Entering | ActorState::Exiting))
    }

    #[cfg(test)]
    pub(crate) fn actor_state(&self, slot: usize) -> ActorState {
        self.actors
            .iter()
            .find(|actor| actor.slot == slot)
            .map_or(ActorState::Offstage, |actor| actor.state)
    }

    #[cfg(test)]
    pub(crate) fn line_states(&self) -> Vec<LineState> {
        self.lines.iter().map(|run| run.state).collect()
    }

    /// Whether sound is on now (unlocked and not muted): a line asks for a clip only then.
    pub(crate) fn set_sound(&mut self, on: bool) {
        self.sound_on = on;
    }

    /// Starts again at the current view with nothing queued: first view, reload, reconnect.
    /// Lines marked `settled` stand on screen at once; the others play from their start.
    pub(crate) fn restart(&mut self, now_ms: u64, plan: ViewPlan<D>) -> Vec<Command<D>> {
        let mut out = Vec::new();
        self.retire_lines(now_ms, "replaced", &mut out);
        self.pending = None;
        self.in_entrance = false;
        self.held.clear();
        self.held_exit = None;
        self.finishing = false;
        self.beat_since_ms = None;
        self.move_battle(now_ms, BattleEvent::Abandon, &mut out);
        for command in self.director.restart() {
            if let DirectorCommand::Render = command {
                out.push(Command::Render);
            }
        }
        self.scene = scene_next(self.scene, SceneEvent::Restore).unwrap_or(self.scene);
        self.scene_since_ms = now_ms;
        out.push(Command::Scene(self.scene));
        self.restore_cast(&plan.cast, now_ms, &mut out);
        self.install_script(plan.script, now_ms, &mut out);
        if plan.diff.enters_combat {
            // The display came up (first view, reload, reconnect, a tab that was hidden) with a
            // fight already under way: the battle machine still enters, so the flat combat screen
            // stays hidden until the battlefield is live (or will not appear).
            self.move_battle(now_ms, BattleEvent::Begin, &mut out);
        }
        self.step(now_ms, &mut out);
        out
    }

    /// Takes a newly accepted view. A scene change only sets the pending successor.
    pub(crate) fn on_view(&mut self, now_ms: u64, plan: ViewPlan<D>) -> Vec<Command<D>> {
        if self.scene == SceneState::Hidden {
            return self.restart(now_ms, plan);
        }
        let mut out = Vec::new();
        let outcome = plan.diff.is_outcome();
        let mut plan = plan;
        if outcome && plan.diff.enters_combat {
            // The view that starts a fight may already carry the enemy's opening turn (it won
            // initiative). That turn is not part of the entrance: it is held and staged as
            // ordinary beats once the battlefield has revealed its banner, initiative roll and
            // roll-call, so nothing of the fight happens behind the cover.
            let opening = ViewDiff {
                roll: plan.diff.roll.take(),
                turn: plan.diff.turn.take(),
                impact: plan.diff.impact.take(),
                defeat: plan.diff.defeat.take(),
                blow_only: true,
                sequence: plan.diff.sequence,
                memorable: plan.diff.memorable,
                ..ViewDiff::default()
            };
            if opening.turn.is_some()
                || opening.roll.is_some()
                || opening.impact.is_some()
                || opening.defeat.is_some()
            {
                self.held.push(opening);
            }
        }
        if outcome {
            self.fade_lines_for_outcome(now_ms, &mut out);
            self.finishing |= plan.diff.banner.is_some();
            // The battlefield's entrance comes first: dice that arrive during it wait.
            let holding = !plan.diff.enters_combat
                && (self.battle == BattleState::Entering
                    || self
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.diff.enters_combat));
            if plan.diff.scene_changed {
                self.pending = Some(Pending {
                    diff: plan.diff.clone(),
                    script: plan.script,
                    cast: plan.cast,
                    arrivals: 1,
                    since_ms: now_ms,
                    submitted: true,
                });
            } else {
                self.install_more(plan.script, now_ms, &mut out);
            }
            if holding {
                if self.held.len() >= HELD_OUTCOMES_CAP {
                    self.held.remove(0);
                }
                self.held.push(plan.diff);
                self.step(now_ms, &mut out);
                return out;
            }
            self.submit_outcome(now_ms, plan.diff, &mut out);
        } else if plan.diff.scene_changed {
            self.set_pending(now_ms, plan);
        } else if let Some(pending) = self.pending.as_mut() {
            // The scene on screen has not left yet: what this view adds belongs to its
            // successor, behind the lines already waiting there.
            pending.script.extend(plan.script);
            pending.cast = plan.cast;
        } else {
            self.install_more(plan.script, now_ms, &mut out);
            let turn_changed = plan.diff.turn_changed;
            let commands = self.director.on_view(now_ms, plan.diff);
            self.absorb(now_ms, commands, &mut out);
            self.follow_turn(turn_changed, now_ms, &mut out);
        }
        self.step(now_ms, &mut out);
        out
    }

    /// The server's active combatant changed and the previous turn's beats are over: the battle
    /// is waiting for the next combatant, not still in the one before.
    fn follow_turn(&mut self, turn_changed: bool, now_ms: u64, out: &mut Vec<Command<D>>) {
        if !turn_changed {
            return;
        }
        if self.director.state() == StageState::Idle {
            self.move_battle(now_ms, BattleEvent::TurnOpen, out);
        } else {
            self.turn_pending = true;
        }
    }

    pub(crate) fn on_tick(&mut self, now_ms: u64) -> Vec<Command<D>> {
        let mut out = Vec::new();
        self.advance_beat_speech(now_ms, &mut out);
        let commands = self.director.on_tick(now_ms);
        self.absorb(now_ms, commands, &mut out);
        self.step(now_ms, &mut out);
        out
    }

    /// The voice player reports about the clip of line `id`.
    pub(crate) fn on_clip(
        &mut self,
        now_ms: u64,
        id: LineId,
        signal: ClipSignal,
    ) -> Vec<Command<D>> {
        let mut out = Vec::new();
        if let Some(run) = self.lines.iter_mut().find(|run| run.id == id) {
            let event = match signal {
                ClipSignal::Loaded => ClipEvent::Loaded,
                ClipSignal::Started { duration_ms } => {
                    run.clip_limit_ms = duration_ms.map_or_else(
                        || run.line.read_ms.saturating_add(CLIP_END_GRACE_MS),
                        |length| length.saturating_add(CLIP_END_GRACE_MS),
                    );
                    ClipEvent::Started
                }
                ClipSignal::Finished => ClipEvent::Finished,
                ClipSignal::Error => ClipEvent::Error,
            };
            if let Some(next) = clip_next(run.clip, event) {
                run.clip = next;
                run.clip_since_ms = now_ms;
                run.timing.at_ms = now_ms;
                match signal {
                    ClipSignal::Started { .. } => run.timing.voice_start_ms = Some(now_ms),
                    ClipSignal::Finished => run.timing.voice_end_ms = Some(now_ms),
                    _ => {}
                }
            }
            let timing = run.timing.clone();
            self.retain_line_timing(timing);
        }
        self.step(now_ms, &mut out);
        out
    }

    // -- scene -------------------------------------------------------------------------------

    /// A scene change sets (or replaces) the one successor. Older unspoken scenes are dropped,
    /// never queued without bound.
    fn set_pending(&mut self, now_ms: u64, plan: ViewPlan<D>) {
        let (arrivals, since_ms, submitted) =
            self.pending.as_ref().map_or((1, now_ms, false), |waiting| {
                (waiting.arrivals + 1, waiting.since_ms, waiting.submitted)
            });
        self.pending = Some(Pending {
            diff: plan.diff,
            script: plan.script,
            cast: plan.cast,
            arrivals,
            since_ms,
            submitted,
        });
    }

    fn catching_up(&self, now_ms: u64) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.arrivals >= 2 || now_ms.saturating_sub(pending.since_ms) > SCENE_WAIT_CAP_MS
        })
    }

    fn move_scene(&mut self, now_ms: u64, event: SceneEvent, out: &mut Vec<Command<D>>) {
        if let Some(next) = scene_next(self.scene, event) {
            self.scene = next;
            self.scene_since_ms = now_ms;
            out.push(Command::Scene(next));
        }
    }

    /// The director began leaving the scene: bubbles go, actors absent from the successor
    /// walk off, and the scene is Exiting.
    fn on_exit_begin(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        self.close_all(now_ms, LINE_CLOSE_MS, out);
        let staying: Vec<usize> = self
            .pending
            .as_ref()
            .map(|pending| pending.cast.clone())
            .unwrap_or_default();
        let leaving: Vec<usize> = self
            .actors
            .iter()
            .filter(|actor| !staying.contains(&actor.slot))
            .map(|actor| actor.slot)
            .collect();
        for slot in leaving {
            self.move_actor(slot, ActorEvent::Leave, now_ms, out);
        }
        self.move_scene(now_ms, SceneEvent::ExitBegin, out);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.diff.enters_combat)
        {
            self.move_battle(now_ms, BattleEvent::Begin, out);
        }
        if self.battle == BattleState::Finishing {
            self.move_battle(now_ms, BattleEvent::FinishDone, out);
        }
    }

    /// The painting changed under the wordmark: the pending successor is now the scene.
    fn on_swap(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        let pending = self.pending.take();
        self.move_scene(now_ms, SceneEvent::ExitDone, out);
        let Some(pending) = pending else {
            return;
        };
        for slot in &pending.cast {
            self.move_actor(*slot, ActorEvent::Cast, now_ms, out);
        }
        self.install_script(pending.script, now_ms, out);
    }

    fn move_battle(&mut self, now_ms: u64, event: BattleEvent, out: &mut Vec<Command<D>>) {
        if let Some(next) = battle_next(self.battle, event) {
            self.battle = next;
            self.battle_since_ms = now_ms;
            if next == BattleState::Entering {
                self.battle_progress_ms = now_ms;
                // Readiness belongs to this fight; a previous fight's failure or completed
                // entrance cannot release this entrance before its renderer reports.
                self.battlefield = Battlefield::Pending;
            }
            out.push(Command::Battle(next));
        }
    }

    /// The fight's entrance is done once the scene and its narration have entered and the
    /// battlefield has shown its banner, initiative roll and roll-call (or will not appear, or
    /// took too long). The dice held back then play, in order.
    fn settle_battle_entrance(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        if self.battle != BattleState::Entering
            || !matches!(
                self.scene,
                SceneState::Presenting | SceneState::AwaitingChoice
            )
        {
            return;
        }
        let waited = now_ms.saturating_sub(self.battle_progress_ms);
        let done = !self.lines_waiting()
            && (!self.battlefield_expected
                || self.battlefield != Battlefield::Pending
                || waited > BATTLE_ENTER_CAP_MS);
        if !done {
            return;
        }
        if self.battlefield_expected
            && self.battlefield == Battlefield::Pending
            && waited > BATTLE_ENTER_CAP_MS
        {
            self.battlefield = Battlefield::Fallback;
        }
        self.move_battle(now_ms, BattleEvent::EntranceDone, out);
        for diff in std::mem::take(&mut self.held) {
            self.submit_outcome(now_ms, diff, out);
        }
    }

    /// Hands an outcome to the cue sequencer. The view that ends a fight is queued behind its own
    /// finishing blow: the dice, the impact and the fall play first, and the scene change (the
    /// victory or defeat scene, its banner and closing card) is held in `held_exit` until the
    /// battlefield has shown its finishing shot out (see [`Self::release_exit`]).
    fn submit_outcome(&mut self, now_ms: u64, mut diff: ViewDiff<D>, out: &mut Vec<Command<D>>) {
        if diff.banner.is_some() && diff.scene_changed {
            let memorable = diff.memorable;
            let blow = ViewDiff {
                roll: diff.roll.take(),
                turn: diff.turn.take(),
                impact: diff.impact.take(),
                defeat: diff.defeat.take(),
                memorable,
                sequence: diff.sequence,
                ..ViewDiff::default()
            };
            self.held_exit = Some(diff);
            self.exit_waiting_since_ms = None;
            let commands = self.director.on_blow(now_ms, blow);
            self.absorb(now_ms, commands, out);
        } else {
            let turn_changed = diff.turn_changed;
            let commands = self.director.on_view(now_ms, diff);
            self.absorb(now_ms, commands, out);
            self.follow_turn(turn_changed, now_ms, out);
        }
    }

    /// Lets the held scene change go once the blow's cues are done and the battlefield is done
    /// with its finishing shot (or will not show one, or [`BATTLE_FINISH_CAP_MS`] passed).
    fn release_exit(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        if self.held_exit.is_none() {
            return;
        }
        // Truly idle: `state()` alone reads Idle while the finishing Roll still waits for
        // the battlefield, which once let the transition and fanfare precede the blow.
        if !self.director.is_idle() {
            self.exit_waiting_since_ms = None;
            return;
        }
        let since = *self.exit_waiting_since_ms.get_or_insert(now_ms);
        let waiting = self.battlefield_expected
            && self.battle != BattleState::Absent
            && self.battlefield == Battlefield::Pending
            && now_ms.saturating_sub(since) <= BATTLE_FINISH_CAP_MS;
        if waiting {
            return;
        }
        if let Some(diff) = self.held_exit.take() {
            self.exit_waiting_since_ms = None;
            let commands = self.director.on_view(now_ms, diff);
            self.absorb(now_ms, commands, out);
        }
    }

    /// Follows the cue sequencer's dice and blows through the battle machine.
    fn follow_battle(&mut self, state: StageState, now_ms: u64, out: &mut Vec<Command<D>>) {
        let closing_blow = self.battle == BattleState::Resolving
            && !matches!(state, StageState::Impact | StageState::Defeat);
        // A roll whose impact was never queued still ends with the director at rest.
        let unresolved_roll = self.battle == BattleState::Acting && state == StageState::Idle;
        if closing_blow || unresolved_roll {
            self.move_battle(now_ms, BattleEvent::TurnClose, out);
        }
        match state {
            StageState::Rolling | StageState::Turn => {
                self.move_battle(now_ms, BattleEvent::TurnOpen, out);
                self.move_battle(now_ms, BattleEvent::ActBegin, out);
            }
            StageState::Impact => self.move_battle(now_ms, BattleEvent::ResolveBegin, out),
            StageState::Defeat if self.finishing => {
                self.move_battle(now_ms, BattleEvent::Finish, out);
            }
            StageState::Aftermath | StageState::ClosingCard => {
                self.finishing = false;
                self.move_battle(now_ms, BattleEvent::FinishDone, out);
                self.move_battle(now_ms, BattleEvent::ExitDone, out);
            }
            _ => {}
        }
    }

    fn entrance_done(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        self.in_entrance = false;
        self.move_scene(now_ms, SceneEvent::EntranceDone, out);
    }

    /// Maps the cue sequencer's commands onto the scene and actor machines.
    fn absorb(
        &mut self,
        now_ms: u64,
        commands: Vec<DirectorCommand<D>>,
        out: &mut Vec<Command<D>>,
    ) {
        for command in commands {
            match command {
                DirectorCommand::Render => out.push(Command::Render),
                DirectorCommand::Skip { detail } => out.push(Command::Skip { detail }),
                DirectorCommand::Rest => {
                    self.follow_battle(StageState::Idle, now_ms, out);
                    if self.turn_pending {
                        self.turn_pending = false;
                        self.move_battle(now_ms, BattleEvent::TurnOpen, out);
                    }
                    if self.in_entrance {
                        self.entrance_done(now_ms, out);
                    }
                    out.push(Command::Rest);
                }
                DirectorCommand::Enter {
                    state,
                    detail,
                    short,
                } => {
                    if self.in_entrance && state != StageState::EnteringScene {
                        self.entrance_done(now_ms, out);
                    }
                    self.follow_battle(state, now_ms, out);
                    match state {
                        StageState::LeavingScene => self.on_exit_begin(now_ms, out),
                        StageState::Interstitial => self.on_swap(now_ms, out),
                        StageState::EnteringScene => self.in_entrance = true,
                        _ => {}
                    }
                    out.push(Command::Enter {
                        state,
                        detail,
                        short,
                    });
                }
            }
        }
    }

    // -- lines -------------------------------------------------------------------------------

    fn make_run(&mut self, line: ScriptLine<D>, now_ms: u64) -> LineRun<D> {
        let id = self.next_id;
        self.next_id += 1;
        let state = if line.settled {
            LineState::Lingering
        } else {
            LineState::Queued
        };
        let mut timing =
            df_observe::presentation::TimingEvent::lifecycle("line", "ready", id, 0, now_ms);
        timing.read_ms = line.read_ms;
        timing.slot = line
            .slot
            .and_then(|slot| u32::try_from(slot).ok())
            .filter(|slot| *slot <= 31);
        timing.opened_ms = (state == LineState::Lingering).then_some(now_ms);
        self.retain_line_timing(timing.clone());
        LineRun {
            id,
            line,
            state,
            clip: ClipState::Silent,
            since_ms: now_ms,
            clip_since_ms: now_ms,
            clip_limit_ms: 0,
            play_sent: false,
            opened_ms: (state == LineState::Lingering).then_some(now_ms),
            close_ms: LINE_CLOSE_MS,
            voiced: false,
            reported_clip: ClipState::Silent,
            intent: None,
            affirmed_ms: now_ms,
            timing,
        }
    }

    /// Takes the speech intents of the view being shown: a loader bubble on each hero who is
    /// typing or talking, kept while the table keeps affirming it, replaced by that
    /// hero's line, closed through the normal exit when the hero stops, and gone after
    /// [`INTENT_EXPIRY_MS`] without a refresh. A loader is not a line: it never delays speech.
    pub(crate) fn on_intents(&mut self, now_ms: u64, marks: Vec<IntentMark<D>>) -> Vec<Command<D>> {
        let mut out = Vec::new();
        for mark in &marks {
            let known = self.lines.iter().position(|run| {
                run.intent.is_some()
                    && run.line.slot == Some(mark.slot)
                    && matches!(run.state, LineState::Composing | LineState::Submitted)
            });
            match known {
                Some(index) => {
                    let Some(run) = self.lines.get_mut(index) else {
                        continue;
                    };
                    run.affirmed_ms = now_ms;
                    let changed = run.intent != Some(mark.kind)
                        && !(run.state == LineState::Submitted
                            && mark.kind != IntentKind::Submitted);
                    if changed {
                        run.intent = Some(mark.kind);
                        run.line.detail = mark.detail.clone();
                        let snapshot = run.clone();
                        self.emit_line(&snapshot, &mut out);
                    }
                    if mark.kind == IntentKind::Submitted {
                        self.set_line(index, LineEvent::Submit, now_ms, &mut out);
                    }
                }
                None => {
                    let line = ScriptLine {
                        detail: mark.detail.clone(),
                        voiceable: false,
                        read_ms: 0,
                        slot: Some(mark.slot),
                        settled: false,
                    };
                    let mut run = self.make_run(line, now_ms);
                    run.state = LineState::Composing;
                    run.intent = Some(mark.kind);
                    self.emit_line(&run, &mut out);
                    self.lines.push(run);
                    if mark.kind == IntentKind::Submitted {
                        let index = self.lines.len() - 1;
                        self.set_line(index, LineEvent::Submit, now_ms, &mut out);
                    }
                }
            }
        }
        // A loader the table no longer names: the hero stopped, or the line has taken its place.
        for index in 0..self.lines.len() {
            let Some(run) = self.lines.get(index) else {
                continue;
            };
            let live = run.intent.is_some()
                && matches!(run.state, LineState::Composing | LineState::Submitted);
            let named = marks.iter().any(|mark| Some(mark.slot) == run.line.slot);
            if live && !named {
                self.retire_intent(index, now_ms, &mut out);
            }
        }
        self.step(now_ms, &mut out);
        out
    }

    /// Whether a line of text for `slot` is waiting its turn or being shown.
    fn line_pending_for(&self, slot: Option<usize>) -> bool {
        self.lines.iter().any(|other| {
            other.intent.is_none()
                && other.line.slot == slot
                && matches!(
                    other.state,
                    LineState::Queued
                        | LineState::Opening
                        | LineState::Speaking
                        | LineState::Lingering
                )
        })
    }

    /// The loader's hero stopped or submitted away: it stays while that hero's line is still
    /// queued, is replaced at once by one already on screen, and otherwise closes normally.
    fn retire_intent(&mut self, index: usize, now_ms: u64, out: &mut Vec<Command<D>>) {
        let Some(run) = self.lines.get(index) else {
            return;
        };
        let slot = run.line.slot;
        let queued = self.lines.iter().any(|other| {
            other.intent.is_none() && other.line.slot == slot && other.state == LineState::Queued
        });
        if queued {
            if self
                .lines
                .get(index)
                .is_some_and(|run| run.state == LineState::Composing)
            {
                self.set_line(index, LineEvent::Submit, now_ms, out);
            }
        } else if self.line_pending_for(slot) {
            self.set_line(index, LineEvent::Replace, now_ms, out);
        } else {
            self.set_line(index, LineEvent::Cancel, now_ms, out);
            self.set_line(index, LineEvent::Close, now_ms, out);
        }
    }

    /// Ends the loaders of `slot`: its line took the place.
    fn replace_intents(&mut self, slot: Option<usize>, now_ms: u64, out: &mut Vec<Command<D>>) {
        for index in 0..self.lines.len() {
            let Some(run) = self.lines.get(index) else {
                continue;
            };
            if run.intent.is_some()
                && run.line.slot == slot
                && matches!(run.state, LineState::Composing | LineState::Submitted)
            {
                self.set_line(index, LineEvent::Replace, now_ms, out);
            }
        }
    }

    /// Loaders the table stopped affirming expire; a submitted one waits for its queued line.
    fn expire_intents(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        for index in 0..self.lines.len() {
            let Some(run) = self.lines.get(index) else {
                continue;
            };
            if run.intent.is_none()
                || !matches!(run.state, LineState::Composing | LineState::Submitted)
            {
                continue;
            }
            let waiting_for_line = self.lines.iter().any(|other| {
                other.intent.is_none()
                    && other.line.slot == run.line.slot
                    && other.state == LineState::Queued
            });
            let silent_for = now_ms.saturating_sub(run.affirmed_ms);
            if !waiting_for_line && silent_for > INTENT_EXPIRY_MS {
                self.retire_intent(index, now_ms, out);
            }
        }
    }

    /// Replaces the scene's lines with a new scene's script.
    fn install_script(
        &mut self,
        script: Vec<ScriptLine<D>>,
        now_ms: u64,
        out: &mut Vec<Command<D>>,
    ) {
        self.retire_lines(now_ms, "scene-change", out);
        self.install_more(script, now_ms, out);
    }

    fn retire_lines(&mut self, now_ms: u64, reason: &'static str, out: &mut Vec<Command<D>>) {
        for mut run in std::mem::take(&mut self.lines) {
            self.cancel_clip(&run, out);
            if run.state != LineState::Done {
                run.timing.at_ms = now_ms;
                run.timing.reason = if run.opened_ms.is_some() {
                    reason
                } else {
                    "skipped"
                };
                if run.opened_ms.is_some() {
                    run.timing.closed_ms = Some(now_ms);
                    run.state = LineState::Done;
                    self.emit_line(&run, out);
                }
                self.retain_line_timing(run.timing);
            }
        }
    }

    /// Adds lines behind the ones waiting.
    fn install_more(&mut self, script: Vec<ScriptLine<D>>, now_ms: u64, out: &mut Vec<Command<D>>) {
        for line in script {
            let run = self.make_run(line, now_ms);
            if run.state == LineState::Lingering {
                self.emit_line(&run, out);
            }
            self.lines.push(run);
        }
    }

    fn emit_line(&self, run: &LineRun<D>, out: &mut Vec<Command<D>>) {
        out.push(Command::Line {
            id: run.id,
            state: run.state,
            detail: run.line.detail.clone(),
            slot: run.line.slot,
        });
    }

    fn cancel_clip(&self, run: &LineRun<D>, out: &mut Vec<Command<D>>) {
        if matches!(
            run.clip,
            ClipState::Requested | ClipState::Ready | ClipState::Playing
        ) {
            out.push(Command::Clip {
                id: run.id,
                op: ClipOp::Cancel,
                detail: run.line.detail.clone(),
            });
        }
    }

    /// Installs the lines that belong to the beat now starting: a blow's cries when its
    /// dice roll, the closing line when the banner rises. The beat owns the stage, so a
    /// line still speaking yields to it; lines of beats that never start are never
    /// installed, so a skipped blow stays silent.
    pub(crate) fn install_beat(
        &mut self,
        now_ms: u64,
        script: Vec<ScriptLine<D>>,
    ) -> Vec<Command<D>> {
        let mut out = Vec::new();
        if script.is_empty() {
            return out;
        }
        self.close_all(now_ms, FIGHT_LINE_FADE_MS, &mut out);
        self.beat_since_ms = Some(now_ms);
        self.install_more(script, now_ms, &mut out);
        self.step(now_ms, &mut out);
        out
    }

    fn advance_beat_speech(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        let Some(since) = self.beat_since_ms else {
            return;
        };
        let elapsed = now_ms.saturating_sub(since);
        let waiting = self.sound_on
            && self.lines.iter().any(|run| {
                (run.clip == ClipState::Playing
                    && now_ms.saturating_sub(run.clip_since_ms)
                        < u64::from(run.clip_limit_ms.saturating_sub(CLIP_END_GRACE_MS)))
                    || (elapsed < BEAT_CLIP_WAIT_MS
                        && matches!(run.clip, ClipState::Requested | ClipState::Ready))
            });
        self.director
            .set_speech_busy(waiting && elapsed < BEAT_SPEECH_CAP_MS);
        let unheard = !waiting
            && self.lines.iter().any(|run| {
                matches!(
                    run.clip,
                    ClipState::Requested | ClipState::Ready | ClipState::Playing
                )
            });
        if elapsed >= BEAT_SPEECH_CAP_MS || unheard {
            self.close_all(now_ms, FIGHT_LINE_FADE_MS, out);
            self.beat_since_ms = None;
        }
    }

    fn set_line(
        &mut self,
        index: usize,
        event: LineEvent,
        now_ms: u64,
        out: &mut Vec<Command<D>>,
    ) -> bool {
        let Some(run) = self.lines.get_mut(index) else {
            return false;
        };
        let Some(next) = line_next(run.state, event) else {
            return false;
        };
        let (before, slot) = (run.state, run.line.slot);
        run.state = next;
        run.since_ms = now_ms;
        if next == LineState::Opening {
            run.opened_ms = Some(now_ms);
            run.timing.opened_ms = Some(now_ms);
        }
        run.timing.at_ms = now_ms;
        if event == LineEvent::SpeechDone {
            // Report which observed gate finished last; short speech must not hide a
            // longer reading window behind a misleading voice-ended reason.
            run.timing.reason =
                if run
                    .timing
                    .voice_end_ms
                    .zip(run.opened_ms)
                    .is_some_and(|(ended, opened)| {
                        ended.saturating_sub(opened) >= u64::from(run.line.read_ms)
                    })
                {
                    "voice-ended"
                } else if run.timing.voice_end_ms.is_none()
                    && run.timing.voice_start_ms.is_some()
                    && run.clip == ClipState::Ended
                {
                    "timeout"
                } else {
                    "read"
                };
        }
        if matches!(next, LineState::Closing | LineState::Done) {
            run.timing.closed_ms = Some(now_ms);
            if event == LineEvent::Close {
                run.timing.reason =
                    if before == LineState::Queued || run.close_ms == FIGHT_LINE_FADE_MS {
                        "skipped"
                    } else {
                        "scene-change"
                    };
            }
        }
        let run = run.clone();
        self.retain_line_timing(run.timing.clone());
        // A queued line closed before it showed never reaches the screen.
        if !(before == LineState::Queued && next == LineState::Done) {
            self.emit_line(&run, out);
        }
        if let Some(slot) = slot {
            match next {
                LineState::Opening => self.move_actor(slot, ActorEvent::LineOpened, now_ms, out),
                LineState::Lingering | LineState::Closing => {
                    let still_speaking = self.lines.iter().any(|other| {
                        other.id != run.id
                            && other.line.slot == Some(slot)
                            && matches!(other.state, LineState::Opening | LineState::Speaking)
                    });
                    if !still_speaking {
                        self.move_actor(slot, ActorEvent::LineClosed, now_ms, out);
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// Fades every line that is not done: queued lines are dropped unshown, the others close.
    fn close_all(&mut self, now_ms: u64, close_ms: u32, out: &mut Vec<Command<D>>) {
        for index in 0..self.lines.len() {
            let Some(run) = self.lines.get_mut(index) else {
                continue;
            };
            run.close_ms = close_ms;
            let snapshot = run.clone();
            if !matches!(snapshot.state, LineState::Done | LineState::Closing) {
                self.cancel_clip(&snapshot, out);
                if let Some(run) = self.lines.get_mut(index)
                    && !run.clip.is_over()
                    && let Some(next) = clip_next(run.clip, ClipEvent::Cancel)
                {
                    run.clip = next;
                }
            }
            self.set_line(index, LineEvent::Close, now_ms, out);
        }
    }

    fn fade_lines_for_outcome(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        self.close_all(now_ms, FIGHT_LINE_FADE_MS, out);
    }

    /// Runs every timer and starts what is due. Idempotent for a given `now_ms`.
    fn step(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        self.watch_scene(now_ms, out);
        self.advance_actors(now_ms, out);
        self.expire_intents(now_ms, out);
        // Catch-up skips queued lines, never the line being read or heard.
        let catching_up = self.catching_up(now_ms);
        for index in 0..self.lines.len() {
            self.advance_line(index, now_ms, catching_up, out);
        }
        self.start_next_line(now_ms, catching_up, out);
        self.prefetch(out);
        self.settle_scene(now_ms, out);
        self.settle_battle_entrance(now_ms, out);
        self.release_exit(now_ms, out);
        self.report_clips(out);
        self.submit_pending(now_ms, out);
    }

    fn watch_scene(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        let waited = now_ms.saturating_sub(self.scene_since_ms);
        match self.scene {
            // A transition that never reports its end takes the normal exit.
            SceneState::Exiting if waited > SCENE_EXIT_CAP_MS => {
                self.on_swap(now_ms, out);
            }
            SceneState::Entering if waited > SCENE_ENTER_CAP_MS => {
                self.entrance_done(now_ms, out);
            }
            _ => {}
        }
    }

    fn advance_actors(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        let due: Vec<(usize, ActorEvent)> = self
            .actors
            .iter()
            .filter_map(|actor| {
                let waited = now_ms.saturating_sub(actor.since_ms);
                match actor.state {
                    ActorState::Entering if waited >= u64::from(ACTOR_ENTER_MS) => {
                        Some((actor.slot, ActorEvent::EnterDone))
                    }
                    ActorState::Exiting if waited >= u64::from(ACTOR_EXIT_MS) => {
                        Some((actor.slot, ActorEvent::ExitDone))
                    }
                    _ => None,
                }
            })
            .collect();
        for (slot, event) in due {
            self.move_actor(slot, event, now_ms, out);
        }
    }

    fn move_actor(
        &mut self,
        slot: usize,
        event: ActorEvent,
        now_ms: u64,
        out: &mut Vec<Command<D>>,
    ) {
        let known = self.actors.iter().position(|actor| actor.slot == slot);
        let index = match known {
            Some(index) => index,
            None => {
                self.actors.push(ActorRun {
                    slot,
                    state: ActorState::Offstage,
                    since_ms: now_ms,
                });
                self.actors.len() - 1
            }
        };
        let Some(actor) = self.actors.get_mut(index) else {
            return;
        };
        if let Some(next) = actor_next(actor.state, event) {
            actor.state = next;
            actor.since_ms = now_ms;
            out.push(Command::Actor { slot, state: next });
        }
    }

    fn restore_cast(&mut self, cast: &[usize], now_ms: u64, out: &mut Vec<Command<D>>) {
        let known: Vec<usize> = self.actors.iter().map(|actor| actor.slot).collect();
        for slot in known {
            if !cast.contains(&slot) {
                self.actors.retain(|actor| actor.slot != slot);
                out.push(Command::Actor {
                    slot,
                    state: ActorState::Offstage,
                });
            }
        }
        for slot in cast {
            self.move_actor(*slot, ActorEvent::Restore, now_ms, out);
        }
    }

    fn advance_line(
        &mut self,
        index: usize,
        now_ms: u64,
        catching_up: bool,
        out: &mut Vec<Command<D>>,
    ) {
        let Some(run) = self.lines.get(index) else {
            return;
        };
        let (state, close_ms) = (run.state, run.close_ms);
        let in_state = now_ms.saturating_sub(run.since_ms);
        let has_next = self.pending.is_some()
            || self
                .lines
                .iter()
                .skip(index + 1)
                .any(|later| later.state == LineState::Queued);
        match state {
            LineState::Opening | LineState::Speaking => {
                if state == LineState::Opening && in_state >= u64::from(LINE_OPEN_MS) {
                    self.set_line(index, LineEvent::OpenDone, now_ms, out);
                }
                self.watch_speech(index, now_ms, out);
            }
            LineState::Lingering => {
                let due = if has_next {
                    let linger = if catching_up {
                        0
                    } else {
                        u64::from(LINE_LINGER_MS)
                    };
                    in_state >= linger
                } else {
                    in_state >= u64::from(FINAL_LINGER_CAP_MS)
                };
                if due {
                    self.set_line(index, LineEvent::LingerDone, now_ms, out);
                }
            }
            LineState::Closing if in_state >= u64::from(close_ms) => {
                self.set_line(index, LineEvent::CloseDone, now_ms, out);
            }
            _ => {}
        }
    }

    /// A line finishes only when its voice is over and its full reading time has passed.
    fn watch_speech(&mut self, index: usize, now_ms: u64, out: &mut Vec<Command<D>>) {
        let Some(run) = self.lines.get_mut(index) else {
            return;
        };
        if run.state != LineState::Opening && run.state != LineState::Speaking {
            return;
        }
        if run.clip == ClipState::Playing
            && now_ms.saturating_sub(run.clip_since_ms) >= u64::from(run.clip_limit_ms)
            && let Some(next) = clip_next(run.clip, ClipEvent::WaitExpired)
        {
            run.clip = next;
        }
        let read = run.opened_ms.is_some_and(|opened| {
            now_ms.saturating_sub(opened.saturating_add(u64::from(LINE_OPEN_MS)))
                >= u64::from(run.line.read_ms)
        });
        if read && (!run.voiced || run.clip.is_over()) {
            self.set_line(index, LineEvent::SpeechDone, now_ms, out);
        }
    }

    /// Starts the next queued line when no other line is being heard or read.
    fn start_next_line(&mut self, now_ms: u64, catching_up: bool, out: &mut Vec<Command<D>>) {
        let presenting = matches!(
            self.scene,
            SceneState::Presenting | SceneState::AwaitingChoice
        );
        if !presenting || self.lines.iter().any(|run| run.state.is_active()) {
            return;
        }
        let Some(index) = self
            .lines
            .iter()
            .position(|run| run.state == LineState::Queued)
        else {
            return;
        };
        if catching_up {
            let Some(snapshot) = self.lines.get(index).cloned() else {
                return;
            };
            self.cancel_clip(&snapshot, out);
            self.set_line(index, LineEvent::Close, now_ms, out);
            out.push(Command::Skip {
                detail: Some(snapshot.line.detail),
            });
            return;
        }
        if self.scene == SceneState::AwaitingChoice {
            self.move_scene(now_ms, SceneEvent::LinesResumed, out);
        }
        let Some(run) = self.lines.get_mut(index) else {
            return;
        };
        let wants_sound = run.line.voiceable && self.sound_on;
        if wants_sound {
            match run.clip {
                ClipState::Silent => {
                    let id = run.id;
                    let detail = run.line.detail.clone();
                    if let Some(next) = clip_next(run.clip, ClipEvent::Request) {
                        run.clip = next;
                        run.clip_since_ms = now_ms;
                    }
                    out.push(Command::Clip {
                        id,
                        op: ClipOp::Request,
                        detail,
                    });
                    return;
                }
                ClipState::Requested => {
                    if now_ms.saturating_sub(run.clip_since_ms) >= u64::from(CLIP_READY_WAIT_MS)
                        && let Some(next) = clip_next(run.clip, ClipEvent::WaitExpired)
                    {
                        run.clip = next;
                    } else {
                        return;
                    }
                }
                ClipState::Ready => {
                    if !run.play_sent {
                        run.play_sent = true;
                        run.clip_since_ms = now_ms;
                        out.push(Command::Clip {
                            id: run.id,
                            op: ClipOp::Play,
                            detail: run.line.detail.clone(),
                        });
                        return;
                    }
                    if now_ms.saturating_sub(run.clip_since_ms) >= u64::from(CLIP_START_WAIT_MS)
                        && let Some(next) = clip_next(run.clip, ClipEvent::WaitExpired)
                    {
                        run.clip = next;
                    } else {
                        return;
                    }
                }
                ClipState::Playing => {
                    run.voiced = true;
                }
                ClipState::Failed | ClipState::Skipped | ClipState::Ended => {}
            }
        }
        let Some(run) = self.lines.get_mut(index) else {
            return;
        };
        let snapshot = run.clone();
        if !snapshot.voiced {
            // Not voiced after all: drop a clip that was still on its way.
            self.cancel_clip(&snapshot, out);
            if let Some(run) = self.lines.get_mut(index)
                && !run.clip.is_over()
                && let Some(next) = clip_next(run.clip, ClipEvent::Cancel)
            {
                run.clip = next;
            }
        }
        let slot = self.lines.get(index).and_then(|run| run.line.slot);
        if self.set_line(index, LineEvent::Begin, now_ms, out) {
            // The hero's line takes the place of the hero's loader.
            self.replace_intents(slot, now_ms, out);
        }
    }

    /// Requests the clips of the next two queued voiceable lines so they are ready at their
    /// turn.
    fn prefetch(&mut self, out: &mut Vec<Command<D>>) {
        if !self.sound_on
            || !matches!(
                self.scene,
                SceneState::Entering | SceneState::Presenting | SceneState::AwaitingChoice
            )
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.arrivals >= 2)
        {
            return;
        }
        for run in self
            .lines
            .iter_mut()
            .filter(|run| run.state == LineState::Queued && run.line.voiceable)
            .take(2)
        {
            if let Some(next) = clip_next(run.clip, ClipEvent::Request) {
                run.clip = next;
                out.push(Command::Clip {
                    id: run.id,
                    op: ClipOp::Request,
                    detail: run.line.detail.clone(),
                });
            }
        }
    }

    /// Moves between Presenting and AwaitingChoice as the lines run out.
    fn settle_scene(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        if self.scene != SceneState::Presenting {
            return;
        }
        let unfinished = self.lines.iter().any(|run| {
            matches!(
                run.state,
                LineState::Queued | LineState::Opening | LineState::Speaking
            )
        });
        if !unfinished {
            self.move_scene(now_ms, SceneEvent::LinesDone, out);
        }
    }

    /// Hands the pending scene change to the cue sequencer once the scene on screen has said
    /// what it had to say: no line is being heard or read and none still waits.
    fn submit_pending(&mut self, now_ms: u64, out: &mut Vec<Command<D>>) {
        let ready = matches!(
            self.scene,
            SceneState::Presenting | SceneState::AwaitingChoice
        ) && !self
            .lines
            .iter()
            .any(|run| run.state.is_active() || run.state == LineState::Queued);
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if !ready || pending.submitted {
            return;
        }
        pending.submitted = true;
        let diff = pending.diff.clone();
        let commands = self.director.on_view(now_ms, diff);
        self.absorb(now_ms, commands, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_machines::{
        BATTLE_ENTER_CAP_MS, BATTLE_FINISH_CAP_MS, CLIP_READY_WAIT_MS, CLIP_START_WAIT_MS,
        LINE_CLOSE_MS, READ_MAXIMUM_MS,
    };

    type TestConductor = Conductor<&'static str>;
    type TestCommand = Command<&'static str>;

    fn line(
        detail: &'static str,
        voiceable: bool,
        slot: Option<usize>,
    ) -> ScriptLine<&'static str> {
        ScriptLine {
            detail,
            voiceable,
            read_ms: 3_000,
            slot,
            settled: false,
        }
    }

    fn scene(
        title: Option<&'static str>,
        script: Vec<ScriptLine<&'static str>>,
        cast: Vec<usize>,
    ) -> ViewPlan<&'static str> {
        ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                title,
                ..ViewDiff::default()
            },
            script,
            cast,
        }
    }

    fn plain(script: Vec<ScriptLine<&'static str>>, cast: Vec<usize>) -> ViewPlan<&'static str> {
        ViewPlan {
            diff: ViewDiff::default(),
            script,
            cast,
        }
    }

    #[test]
    fn line_timing_preserves_actual_voice_end_and_reading_budget_after_close() {
        for (voice_end, reason) in [(500, "read"), (4_000, "voice-ended")] {
            let mut conductor = Conductor::new();
            let run = conductor.make_run(line("public line", true, Some(2)), 0);
            let id = run.id;
            conductor.lines.push(run);
            let mut commands = Vec::new();
            conductor.set_line(0, LineEvent::Begin, 100, &mut commands);
            conductor.lines[0].clip = ClipState::Requested;
            conductor.on_clip(
                200,
                id,
                ClipSignal::Started {
                    duration_ms: Some(300),
                },
            );
            conductor.on_clip(voice_end, id, ClipSignal::Finished);
            conductor.set_line(0, LineEvent::SpeechDone, 4_000, &mut commands);
            conductor.set_line(0, LineEvent::LingerDone, 6_000, &mut commands);
            let timing = conductor.line_timing(id).unwrap();
            assert_eq!(timing.opened_ms, Some(100));
            assert_eq!(timing.voice_start_ms, Some(200));
            assert_eq!(timing.voice_end_ms, Some(voice_end));
            assert_eq!(timing.closed_ms, Some(6_000));
            assert_eq!(timing.read_ms, 3_000);
            assert_eq!(timing.reason, reason);
        }
    }

    #[test]
    fn voice_watchdog_timing_never_fabricates_a_media_end() {
        let mut conductor = Conductor::new();
        let run = conductor.make_run(line("public line", true, None), 0);
        let id = run.id;
        conductor.lines.push(run);
        let mut commands = Vec::new();
        conductor.set_line(0, LineEvent::Begin, 100, &mut commands);
        conductor.lines[0].clip = ClipState::Requested;
        conductor.lines[0].voiced = true;
        conductor.on_clip(
            200,
            id,
            ClipSignal::Started {
                duration_ms: Some(300),
            },
        );
        conductor.advance_line(0, 10_000, false, &mut commands);
        conductor.set_line(0, LineEvent::LingerDone, 11_000, &mut commands);
        let timing = conductor.line_timing(id).unwrap();
        assert_eq!(timing.voice_start_ms, Some(200));
        assert_eq!(timing.voice_end_ms, None);
        assert_eq!(timing.reason, "timeout");
    }

    #[test]
    fn restart_reports_the_previous_visible_lines_closure_before_replacing_it() {
        let mut conductor = Conductor::new();
        let run = conductor.make_run(line("public line", false, None), 0);
        let id = run.id;
        conductor.lines.push(run);
        conductor.set_line(0, LineEvent::Begin, 100, &mut Vec::new());
        let commands = conductor.restart(200, plain(Vec::new(), Vec::new()));
        assert!(commands.iter().any(|command| matches!(command, Command::Line { id: closed, state: LineState::Done, .. } if *closed == id)));
        let timing = conductor.line_timing(id).unwrap();
        assert_eq!(timing.closed_ms, Some(200));
        assert_eq!(timing.reason, "replaced");
    }

    /// A scripted voice: a requested clip loads after `load_ms`; a played clip starts after
    /// `start_ms` and lasts `length_ms`. `None` for a stage withholds that signal.
    #[derive(Clone, Copy)]
    struct Voice {
        load_ms: Option<u64>,
        start_ms: Option<u64>,
        length_ms: Option<u64>,
        reported_length_ms: Option<u32>,
    }

    impl Voice {
        fn working(length_ms: u64) -> Self {
            Self {
                load_ms: Some(100),
                start_ms: Some(50),
                length_ms: Some(length_ms),
                reported_length_ms: u32::try_from(length_ms).ok(),
            }
        }
    }

    /// Runs the conductor in 50 ms steps with a scripted voice answering its clip commands,
    /// and records every command with the time it was issued.
    struct Harness {
        conductor: TestConductor,
        voice: Voice,
        due: Vec<(u64, LineId, ClipSignal)>,
        log: Vec<(u64, TestCommand)>,
        now: u64,
    }

    impl Harness {
        /// Starts with an empty lobby already shown, cast `cast`.
        fn new(
            voice: Voice,
            sound: bool,
            script: Vec<ScriptLine<&'static str>>,
            cast: Vec<usize>,
        ) -> Self {
            let mut conductor = TestConductor::new();
            conductor.set_sound(sound);
            let mut harness = Self {
                conductor,
                voice,
                due: Vec::new(),
                log: Vec::new(),
                now: 0,
            };
            let commands = harness.conductor.restart(0, plain(script, cast));
            harness.record(commands);
            harness
        }

        fn record(&mut self, commands: Vec<TestCommand>) {
            for command in &commands {
                if let Command::Clip { id, op, .. } = command {
                    match op {
                        ClipOp::Request => {
                            if let Some(load) = self.voice.load_ms {
                                self.due.push((self.now + load, *id, ClipSignal::Loaded));
                            }
                        }
                        ClipOp::Play => {
                            if let Some(start) = self.voice.start_ms {
                                self.due.push((
                                    self.now + start,
                                    *id,
                                    ClipSignal::Started {
                                        duration_ms: self.voice.reported_length_ms,
                                    },
                                ));
                                if let Some(length) = self.voice.length_ms {
                                    self.due.push((
                                        self.now + start + length,
                                        *id,
                                        ClipSignal::Finished,
                                    ));
                                }
                            }
                        }
                        ClipOp::Cancel => self.due.retain(|(_, due_id, _)| due_id != id),
                    }
                }
            }
            let now = self.now;
            self.log
                .extend(commands.into_iter().map(|command| (now, command)));
        }

        fn view(&mut self, plan: ViewPlan<&'static str>) {
            let commands = self.conductor.on_view(self.now, plan);
            self.record(commands);
        }

        /// Advances to `until_ms`, delivering voice signals and ticks.
        fn run_to(&mut self, until_ms: u64) {
            while self.now < until_ms {
                self.now += 50;
                let mut ready: Vec<(u64, LineId, ClipSignal)> = Vec::new();
                let now = self.now;
                self.due.retain(|entry| {
                    if entry.0 <= now {
                        ready.push(*entry);
                        false
                    } else {
                        true
                    }
                });
                ready.sort_by_key(|entry| entry.0);
                for (_, id, signal) in ready {
                    let commands = self.conductor.on_clip(self.now, id, signal);
                    self.record(commands);
                }
                let commands = self.conductor.on_tick(self.now);
                self.record(commands);
                let active = self
                    .conductor
                    .line_states()
                    .iter()
                    .filter(|state| state.is_active())
                    .count();
                assert!(
                    active <= 1,
                    "two lines active at {}: {:?}",
                    self.now,
                    self.conductor.line_states()
                );
            }
        }

        fn first_time(&self, wanted: impl Fn(&TestCommand) -> bool) -> Option<u64> {
            self.log
                .iter()
                .find(|(_, command)| wanted(command))
                .map(|(at, _)| *at)
        }

        fn line_at(&self, detail: &'static str, state: LineState) -> Option<u64> {
            self.first_time(|command| {
                matches!(command, Command::Line { state: s, detail: d, .. } if *s == state && *d == detail)
            })
        }

        fn opened(&self, detail: &'static str) -> Option<u64> {
            self.line_at(detail, LineState::Opening)
        }

        fn closing(&self, detail: &'static str) -> Option<u64> {
            self.line_at(detail, LineState::Closing)
        }

        fn play_at(&self, detail: &'static str) -> Option<u64> {
            self.first_time(|command| {
                matches!(command, Command::Clip { op: ClipOp::Play, detail: d, .. } if *d == detail)
            })
        }

        fn rendered(&self) -> Vec<u64> {
            self.log
                .iter()
                .filter(|(_, command)| matches!(command, Command::Render))
                .map(|(at, _)| *at)
                .collect()
        }

        fn entered_after(&self, wanted: StageState, after_ms: u64) -> Option<u64> {
            self.log
                .iter()
                .find(|(at, command)| {
                    *at >= after_ms
                        && matches!(command, Command::Enter { state, .. } if *state == wanted)
                })
                .map(|(at, _)| *at)
        }

        fn entered(&self, wanted: StageState) -> Option<u64> {
            self.first_time(
                |command| matches!(command, Command::Enter { state, .. } if *state == wanted),
            )
        }

        /// Steps the bare conductor without voice signals.
        fn focus(&self) -> Option<LineState> {
            self.conductor.focus_line().map(|(_, state)| state)
        }
    }

    #[test]
    fn live_enemy_queue_keeps_ticking_while_the_battlefield_finishes_an_impact() {
        let mut conductor = TestConductor::new();
        conductor.restart(0, plain(Vec::new(), Vec::new()));
        for (sequence, enemy) in [(3, "Weasel 1"), (4, "Weasel 2"), (5, "Weasel 3")] {
            conductor.on_view(
                0,
                ViewPlan {
                    diff: ViewDiff {
                        roll: Some(enemy),
                        impact: Some(enemy),
                        blow_only: true,
                        sequence,
                        ..ViewDiff::default()
                    },
                    script: Vec::new(),
                    cast: Vec::new(),
                },
            );
        }
        conductor.set_battlefield_busy(true);
        conductor.on_tick(1_150);
        conductor.on_tick(1_800);
        assert!(
            conductor.is_busy(),
            "queued rolls still need the adapter clock"
        );
    }

    #[test]
    fn live_recorded_three_enemy_outcomes_keep_every_phase_and_bound_the_gap() {
        let outcomes: Vec<_> = [(3, "Weasel 1"), (4, "Weasel 2"), (5, "Weasel 3")]
            .into_iter()
            .map(|(sequence, name)| df_protocol::common::CombatOutcome {
                sequence,
                actor_name: name.to_owned(),
                ..Default::default()
            })
            .collect();
        let refs: Vec<_> = outcomes.iter().collect();
        let marks =
            crate::presentation::turn_marks(&refs, &Default::default(), &Default::default());
        assert!(marks.iter().all(|mark| mark.turn_start));
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), Vec::new());
        harness.conductor.observe_presented(2);
        let names = ["Weasel 1", "Weasel 2", "Weasel 3"];
        for (outcome, name) in outcomes.iter().zip(names) {
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some(name),
                    impact: Some(name),
                    blow_only: true,
                    sequence: outcome.sequence,
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: Vec::new(),
            });
        }
        let mut busy_until = 5_000;
        while harness.now < 20_000 && harness.conductor.is_busy() {
            harness
                .conductor
                .set_battlefield_busy(harness.now < busy_until);
            let before = harness.log.len();
            harness.run_to(harness.now + 50);
            if harness.log[before..].iter().any(|(_, command)| {
                matches!(
                    command,
                    Command::Enter {
                        state: StageState::Rolling,
                        ..
                    }
                )
            }) {
                busy_until = harness.now + 5_000;
            }
        }
        let phases: Vec<_> = harness
            .conductor
            .beat_log()
            .iter()
            .map(|record| (record.sequence, record.kind))
            .collect();
        assert_eq!(
            phases,
            [
                (3, "rolling"),
                (3, "impact"),
                (4, "rolling"),
                (4, "impact"),
                (5, "rolling"),
                (5, "impact")
            ]
        );
        let headings: Vec<_> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state: StageState::Rolling,
                    detail,
                    ..
                } => *detail,
                _ => None,
            })
            .collect();
        assert_eq!(headings, names);
        assert!(
            !harness.log.iter().any(|(_, command)| matches!(
                command,
                Command::Enter {
                    state: StageState::Rolling | StageState::Impact,
                    short: true,
                    ..
                }
            )),
            "ordinary enemy phases keep normal-speed rolls and impacts"
        );
        let records: Vec<_> = harness.conductor.beat_log().iter().collect();
        for pair in records.windows(2) {
            if pair[0].kind == "impact" {
                assert!(pair[1].start_ms.saturating_sub(pair[0].end_ms) <= 1_500);
            }
        }
        assert_eq!(harness.conductor.presented_sequence(), 5);
    }

    #[test]
    fn live_a_voice_without_an_end_signal_holds_the_next_blow_only_to_the_cap() {
        for (length_ms, before_ms, deadline_ms) in [
            (120_000, 9_000, BEAT_SPEECH_CAP_MS + 50),
            (1_000, 1_700, 1_850),
            (4_000, 3_900, 4_250),
        ] {
            let voice = Voice {
                length_ms: None,
                reported_length_ms: Some(length_ms),
                ..Voice::working(1_000)
            };
            let mut harness = Harness::new(voice, true, Vec::new(), Vec::new());
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some("first"),
                    impact: Some("first"),
                    sequence: 3,
                    blow_only: true,
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: Vec::new(),
            });
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some("second"),
                    impact: Some("second"),
                    sequence: 4,
                    blow_only: true,
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: Vec::new(),
            });
            let commands = harness
                .conductor
                .install_beat(0, vec![line("cry", true, None)]);
            harness.record(commands);
            harness.run_to(before_ms);
            assert!(!harness.log.iter().any(|(_, command)| matches!(
                command,
                Command::Enter {
                    state: StageState::Rolling,
                    detail: Some("second"),
                    ..
                }
            )));
            harness.run_to(deadline_ms + 1_800);
            let second = harness
                .log
                .iter()
                .find_map(|(at, command)| match command {
                    Command::Enter {
                        state: StageState::Rolling,
                        detail: Some("second"),
                        ..
                    } => Some(*at),
                    _ => None,
                })
                .expect("released at the speech cap");
            assert!(second <= deadline_ms);
            assert!(harness.closing("cry").is_some());
        }
    }

    #[test]
    fn live_muted_failed_and_unavailable_clips_do_not_hold_the_next_blow() {
        for (sound, fails) in [(false, false), (true, true), (true, false)] {
            let mut harness = Harness::new(
                Voice {
                    load_ms: None,
                    ..Voice::working(1_000)
                },
                sound,
                Vec::new(),
                Vec::new(),
            );
            for (sequence, enemy) in [(3, "first"), (4, "second")] {
                harness.view(ViewPlan {
                    diff: ViewDiff {
                        roll: Some(enemy),
                        impact: Some(enemy),
                        sequence,
                        blow_only: true,
                        ..ViewDiff::default()
                    },
                    script: Vec::new(),
                    cast: Vec::new(),
                });
            }
            let commands = harness
                .conductor
                .install_beat(0, vec![line("cry", true, None)]);
            let id = commands.iter().find_map(|command| match command {
                Command::Clip {
                    id,
                    op: ClipOp::Request,
                    ..
                } => Some(*id),
                _ => None,
            });
            harness.record(commands);
            if fails {
                let commands = harness
                    .conductor
                    .on_clip(100, id.unwrap(), ClipSignal::Error);
                harness.record(commands);
            }
            harness.run_to(3_500);
            let second = harness
                .log
                .iter()
                .find_map(|(at, command)| match command {
                    Command::Enter {
                        state: StageState::Rolling,
                        detail: Some("second"),
                        ..
                    } => Some(*at),
                    _ => None,
                })
                .expect("no wait for absent audio");
            assert!(second <= 3_300, "sound={sound}, fails={fails}: {second}");
        }
    }

    #[test]
    fn live_a_recorded_move_only_turn_has_a_heading_and_no_invented_roll() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), Vec::new());
        harness.view(ViewPlan {
            diff: ViewDiff {
                turn: Some("moving enemy"),
                sequence: 3,
                blow_only: true,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: Vec::new(),
        });
        harness.view(ViewPlan {
            diff: ViewDiff {
                roll: Some("attacking enemy"),
                impact: Some("attacking enemy"),
                sequence: 4,
                blow_only: true,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: Vec::new(),
        });
        harness.run_to(4_000);
        let cues: Vec<_> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state,
                    detail: Some(actor),
                    ..
                } => Some((*state, *actor)),
                _ => None,
            })
            .collect();
        assert_eq!(
            cues,
            [
                (StageState::Turn, "moving enemy"),
                (StageState::Rolling, "attacking enemy"),
                (StageState::Impact, "attacking enemy")
            ]
        );
        assert_eq!(harness.conductor.presented_sequence(), 4);
    }

    #[test]
    fn beat_lines_open_with_their_beat_and_yield_to_the_next() {
        let mut conductor = TestConductor::new();
        conductor.set_sound(false);
        let _ = conductor.restart(0, plain(Vec::new(), Vec::new()));
        // Lines of a beat that never starts are never installed: nothing reaches the
        // screen for a silenced beat.
        assert!(conductor.install_beat(0, Vec::new()).is_empty());
        let installed = conductor.install_beat(0, vec![line("first cry", false, None)]);
        assert!(
            installed.iter().any(|command| matches!(
                command,
                Command::Line {
                    state: LineState::Opening,
                    detail: "first cry",
                    ..
                }
            )),
            "the beat's cry opens with the beat"
        );
        // The next beat takes the stage while the first cry is still speaking: the old
        // line closes and the new one opens, never overlapping.
        let taken = conductor.install_beat(1_000, vec![line("second cry", false, None)]);
        assert!(
            taken.iter().any(|command| matches!(
                command,
                Command::Line {
                    state: LineState::Closing,
                    detail: "first cry",
                    ..
                }
            )),
            "the speaking cry yields to the new beat"
        );
        assert!(
            taken.iter().any(|command| matches!(
                command,
                Command::Line {
                    state: LineState::Opening,
                    detail: "second cry",
                    ..
                }
            )),
            "the new beat's cry opens"
        );
    }

    fn opening_script() -> Vec<ScriptLine<&'static str>> {
        vec![
            line("narration", true, None),
            line("courier", true, Some(2)),
        ]
    }

    #[test]
    fn the_opening_scene_opens_each_line_when_its_own_clip_starts() {
        let mut harness = Harness::new(Voice::working(3_000), true, Vec::new(), vec![0, 1]);
        harness.view(scene(Some("title"), opening_script(), vec![0, 1, 2]));
        harness.run_to(20_000);
        let narration_opened = harness.opened("narration").expect("narration opens");
        let courier_opened = harness.opened("courier").expect("courier opens");
        let narration_play = harness.play_at("narration").expect("narration played");
        let courier_play = harness.play_at("courier").expect("courier played");
        // Each bubble opens as its clip starts (50 ms after the play command), never before.
        assert_eq!(narration_opened, narration_play + 50);
        assert_eq!(courier_opened, courier_play + 50);
        // The narrator comes first, and the character only after the narrator's clip and its
        // lingering have run.
        assert!(courier_opened >= narration_opened + 3_000 + u64::from(LINE_LINGER_MS));
        // The entrance (wordmark, title card, scene) finishes before the first line opens.
        let entering = harness
            .entered(StageState::EnteringScene)
            .expect("entering");
        assert!(narration_opened + 50 >= entering + 800);
    }

    #[test]
    fn a_choice_made_mid_speech_leaves_the_old_scene_until_its_line_ends() {
        let mut harness = Harness::new(
            Voice::working(6_000),
            true,
            vec![line("old narration", true, None)],
            vec![0],
        );
        harness.run_to(1_000);
        assert_eq!(harness.focus(), Some(LineState::Speaking));
        harness.view(scene(
            None,
            vec![line("new narration", true, None)],
            vec![0],
        ));
        assert!(harness.conductor.has_pending());
        harness.run_to(2_500);
        // A frame taken now still shows the old scene with its line in progress.
        assert!(harness.rendered().len() == 1, "{:?}", harness.rendered());
        assert_eq!(harness.conductor.scene_state(), SceneState::Presenting);
        assert_eq!(harness.focus(), Some(LineState::Speaking));
        harness.run_to(20_000);
        let clip_end = harness
            .line_at("old narration", LineState::Lingering)
            .expect("old line ends");
        let swap = harness.rendered()[1];
        assert!(
            swap >= clip_end + u64::from(LINE_LINGER_MS),
            "swap {swap} before end {clip_end}"
        );
        assert!(harness.opened("new narration").expect("new scene speaks") > swap);
    }

    #[test]
    fn short_voice_waits_for_the_whole_reading_time_before_lingering() {
        let mut spoken = line("a reply read from across the table", true, Some(2));
        spoken.read_ms = 12_000;
        let mut harness = Harness::new(Voice::working(1_000), true, vec![spoken], vec![0, 2]);
        harness.run_to(2_000);
        let opened = harness
            .opened("a reply read from across the table")
            .unwrap();
        assert_eq!(harness.focus(), Some(LineState::Speaking));
        harness.run_to(15_000);
        let lingered = harness
            .line_at("a reply read from across the table", LineState::Lingering)
            .unwrap();
        assert!(lingered >= opened + 12_000);
    }

    #[test]
    fn a_long_reply_keeps_the_next_line_queued_until_the_reading_budget_and_linger_finish() {
        let mut first = line("a long reply", false, Some(0));
        first.read_ms = READ_MAXIMUM_MS;
        let second = line("the next reply", false, Some(1));
        let mut harness = Harness::new(Voice::working(1_000), false, vec![first], vec![0, 1]);
        harness.run_to(1_000);
        harness.view(plain(vec![second], vec![0, 1]));
        harness.run_to(50_000);
        let first_open = harness.opened("a long reply").expect("long reply opens");
        let second_open = harness.opened("the next reply").expect("next reply opens");
        assert!(
            second_open >= first_open + u64::from(READ_MAXIMUM_MS + LINE_LINGER_MS),
            "successor opened before the human reading window elapsed: {first_open} -> {second_open}"
        );
    }

    #[test]
    fn an_overdue_successor_does_not_cut_a_long_voice_in_progress() {
        let mut harness = Harness::new(
            Voice::working(40_000),
            true,
            vec![line("long voice", true, None)],
            vec![0],
        );
        harness.run_to(1_000);
        harness.view(scene(None, vec![line("successor", false, None)], vec![0]));
        harness.run_to(35_000);
        assert!(harness.line_at("long voice", LineState::Closing).is_none());
        assert!(harness.opened("successor").is_none());
        harness.run_to(50_000);
        assert!(harness.line_at("long voice", LineState::Closing).unwrap() >= 40_000);
        assert!(harness.opened("successor").is_some());
    }

    #[test]
    fn three_quick_actions_converge_on_the_newest_scene_within_the_bound() {
        let mut harness = Harness::new(
            Voice::working(6_000),
            true,
            vec![
                line("old narration", true, None),
                line("old reply", true, Some(2)),
            ],
            vec![0, 2],
        );
        harness.run_to(1_000);
        harness.view(scene(
            None,
            vec![line("first narration", true, None)],
            vec![0],
        ));
        harness.run_to(1_100);
        harness.view(scene(
            None,
            vec![line("second narration", true, None)],
            vec![0],
        ));
        harness.run_to(1_200);
        harness.view(scene(
            None,
            vec![line("third narration", true, None)],
            vec![0],
        ));
        harness.run_to(40_000);
        // Nothing from the skipped scenes is ever shown.
        assert_eq!(harness.opened("first narration"), None);
        assert_eq!(harness.opened("second narration"), None);
        let third = harness
            .opened("third narration")
            .expect("the newest scene speaks");
        // The old narration finishes; the queued reply goes to the log without flashing.
        let narration_end = harness
            .line_at("old narration", LineState::Lingering)
            .expect("old narration ends");
        assert!(
            narration_end >= 6_000,
            "the old clip is not cut: {narration_end}"
        );
        assert!(harness.opened("old reply").is_none());
        assert!(harness.log.iter().any(|(_, command)| {
            matches!(
                command,
                Command::Skip {
                    detail: Some("old reply")
                }
            )
        }));
        assert!(harness.play_at("old reply").is_none());
        assert!(third <= narration_end + 12_000, "converged at {third}");
        assert_eq!(
            harness.rendered().len(),
            2,
            "the newest scene is drawn once"
        );
    }

    #[test]
    fn muted_runs_the_same_order_on_reading_time_without_any_clip() {
        let mut harness = Harness::new(Voice::working(3_000), false, Vec::new(), vec![0, 1]);
        harness.view(scene(Some("title"), opening_script(), vec![0, 1, 2]));
        harness.run_to(30_000);
        assert!(
            !harness
                .log
                .iter()
                .any(|(_, command)| matches!(command, Command::Clip { .. })),
            "no clip is requested without sound"
        );
        let narration = harness.opened("narration").expect("narration");
        let courier = harness.opened("courier").expect("courier");
        assert!(narration < courier);
        // Reading time (3 s) plus the open and linger: comparable to a 3 s clip.
        let gap = courier - narration;
        assert!(
            (3_000..=3_000 + u64::from(LINE_OPEN_MS) + u64::from(LINE_LINGER_MS) + 200)
                .contains(&gap),
            "{gap}"
        );
    }

    #[test]
    fn a_fight_starting_mid_speech_fades_the_line_within_the_bound_and_is_not_held() {
        let mut harness = Harness::new(
            Voice::working(8_000),
            true,
            vec![line("story", true, None)],
            vec![0],
        );
        harness.run_to(1_000);
        let before = harness.now;
        harness.view(ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                enters_combat: true,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0],
        });
        harness.run_to(before + 1_000);
        let leaving = harness
            .entered(StageState::LeavingScene)
            .expect("the fight is not held");
        assert_eq!(leaving, before);
        let closed = harness.closing("story").expect("line fades");
        assert!(closed - before <= u64::from(FIGHT_LINE_FADE_MS));
        assert!(harness.log.iter().any(|(_, command)| matches!(
            command,
            Command::Clip {
                op: ClipOp::Cancel,
                detail: "story",
                ..
            }
        )));
        harness.run_to(before + 2_000);
        assert!(
            harness
                .conductor
                .line_states()
                .iter()
                .all(|state| *state == LineState::Done)
        );
    }

    #[test]
    fn a_clip_that_never_loads_falls_back_to_reading_time() {
        let voice = Voice {
            load_ms: None,
            ..Voice::working(3_000)
        };
        let mut harness = Harness::new(voice, true, Vec::new(), vec![0]);
        harness.view(scene(None, vec![line("narration", true, None)], vec![0]));
        harness.run_to(u64::from(CLIP_READY_WAIT_MS) + 8_000);
        let opened = harness.opened("narration").expect("opens anyway");
        let requested = harness
            .first_time(|command| {
                matches!(
                    command,
                    Command::Clip {
                        op: ClipOp::Request,
                        ..
                    }
                )
            })
            .expect("requested");
        assert!(opened >= requested + u64::from(CLIP_READY_WAIT_MS));
        assert!(opened <= requested + u64::from(CLIP_READY_WAIT_MS) + 200);
        // The line is reported as having no clip, not as spoken.
        assert!(harness.log.iter().any(|(_, command)| matches!(
            command,
            Command::ClipState {
                state: ClipState::Failed,
                ..
            }
        )));
    }

    #[test]
    fn a_slow_reply_clip_is_waited_for_and_the_reply_opens_when_it_starts() {
        // The clip arrives 9 s after it was requested, longer than the old 5 s race.
        let voice = Voice {
            load_ms: Some(9_000),
            ..Voice::working(3_000)
        };
        let mut harness = Harness::new(voice, true, Vec::new(), vec![0, 2]);
        harness.view(plain(vec![line("npc reply", true, Some(2))], vec![0, 2]));
        harness.run_to(5_000);
        assert!(
            harness.opened("npc reply").is_none(),
            "not raced by a timer"
        );
        assert_eq!(harness.conductor.awaiting_clip_slot(), Some(2));
        harness.run_to(20_000);
        let opened = harness.opened("npc reply").expect("opens");
        let played = harness.play_at("npc reply").expect("played");
        assert_eq!(opened, played + 50);
        assert!(harness.log.iter().any(|(_, command)| matches!(
            command,
            Command::ClipState {
                state: ClipState::Ended,
                ..
            }
        )));
        assert_eq!(harness.conductor.awaiting_clip_slot(), None);
    }

    #[test]
    fn five_consecutive_replies_each_reach_playing_and_ended() {
        let mut harness = Harness::new(Voice::working(2_000), true, Vec::new(), vec![0, 2]);
        let replies = ["reply 1", "reply 2", "reply 3", "reply 4", "reply 5"];
        for reply in replies {
            harness.view(plain(vec![line("hero", false, Some(0))], vec![0, 2]));
            harness.view(plain(vec![line(reply, true, Some(2))], vec![0, 2]));
            harness.run_to(harness.now + 19_000);
        }
        for reply in replies {
            let id = harness
                .log
                .iter()
                .find_map(|(_, command)| match command {
                    Command::Clip {
                        id,
                        op: ClipOp::Play,
                        detail,
                    } if *detail == reply => Some(*id),
                    _ => None,
                })
                .expect("each reply's clip was played");
            let states: Vec<ClipState> = harness
                .log
                .iter()
                .filter_map(|(_, command)| match command {
                    Command::ClipState { id: other, state } if *other == id => Some(*state),
                    _ => None,
                })
                .collect();
            assert!(
                states.contains(&ClipState::Playing) && states.contains(&ClipState::Ended),
                "{reply}: {states:?}"
            );
            assert!(harness.opened(reply).is_some());
        }
    }

    #[test]
    fn a_blocked_play_that_never_starts_times_out() {
        let voice = Voice {
            start_ms: None,
            ..Voice::working(3_000)
        };
        let mut harness = Harness::new(voice, true, Vec::new(), vec![0]);
        harness.view(scene(None, vec![line("narration", true, None)], vec![0]));
        harness.run_to(20_000);
        let played = harness.play_at("narration").expect("play commanded");
        let opened = harness.opened("narration").expect("falls back");
        assert!(opened >= played + u64::from(CLIP_START_WAIT_MS));
        assert!(opened <= played + u64::from(CLIP_START_WAIT_MS) + 200);
    }

    #[test]
    fn a_clip_whose_end_never_arrives_ends_after_its_length_and_grace() {
        let voice = Voice {
            length_ms: None,
            ..Voice::working(2_000)
        };
        let mut harness = Harness::new(voice, true, Vec::new(), vec![0]);
        harness.view(scene(None, vec![line("narration", true, None)], vec![0]));
        harness.run_to(20_000);
        let opened = harness.opened("narration").expect("opens");
        let ended = harness
            .line_at("narration", LineState::Lingering)
            .expect("ends");
        assert!(ended - opened >= 2_000 + u64::from(CLIP_END_GRACE_MS) - 100);
        assert!(ended - opened <= 2_000 + u64::from(CLIP_END_GRACE_MS) + 200);
    }

    #[test]
    fn a_missing_voice_end_uses_its_clip_bound_instead_of_cutting_at_the_scene_cap() {
        let voice = Voice {
            length_ms: None,
            reported_length_ms: Some(300_000),
            ..Voice::working(1_000)
        };
        let mut harness = Harness::new(voice, true, vec![line("old", true, None)], vec![0]);
        harness.run_to(500);
        harness.view(scene(None, vec![line("new", true, None)], vec![0]));
        let waiting_since = harness.now;
        harness.run_to(waiting_since + SCENE_WAIT_CAP_MS + 3_000);
        assert_eq!(harness.rendered().len(), 1);
        assert!(harness.line_at("old", LineState::Closing).is_none());
        let opened = harness.opened("old").unwrap();
        harness.run_to(opened + 300_000 + u64::from(CLIP_END_GRACE_MS) + 3_000);
        let swap = *harness
            .rendered()
            .get(1)
            .expect("voice watchdog releases scene");
        assert!(swap >= opened + 300_000 + u64::from(CLIP_END_GRACE_MS));
        assert!(swap <= opened + 300_000 + u64::from(CLIP_END_GRACE_MS) + 2_000);
    }

    #[test]
    fn two_heroes_speaking_two_seconds_apart_queue_for_full_reading_turns() {
        let mut first = line("Brakka asks", false, Some(0));
        first.read_ms = 6_000;
        let mut second = line("Sigrun asks", false, Some(1));
        second.read_ms = 4_000;
        let mut harness = Harness::new(Voice::working(1_000), false, vec![first], vec![0, 1]);
        harness.run_to(2_000);
        harness.view(plain(vec![second], vec![0, 1]));
        harness.run_to(20_000);
        let first_open = harness.opened("Brakka asks").unwrap();
        let first_close = harness.line_at("Brakka asks", LineState::Closing).unwrap();
        let second_open = harness.opened("Sigrun asks").unwrap();
        let second_read = harness
            .line_at("Sigrun asks", LineState::Lingering)
            .unwrap();
        assert!(first_close >= first_open + 6_000);
        assert!(second_open >= first_close);
        assert!(second_read >= second_open + 4_000);
    }

    #[test]
    fn a_reply_to_a_typed_line_waits_behind_it_in_order() {
        let mut harness = Harness::new(Voice::working(2_000), true, Vec::new(), vec![0, 2]);
        harness.view(plain(vec![line("hero line", false, Some(0))], vec![0, 2]));
        harness.run_to(500);
        harness.view(plain(vec![line("npc reply", true, Some(2))], vec![0, 2]));
        harness.run_to(20_000);
        let hero = harness.opened("hero line").expect("hero line");
        let reply = harness.opened("npc reply").expect("reply");
        assert!(hero < reply);
        assert!(
            harness.closing("hero line").expect("hero closes") <= reply,
            "the reply replaces the hero's bubble"
        );
        assert_eq!(harness.conductor.scene_state(), SceneState::AwaitingChoice);
    }

    #[test]
    fn actors_absent_from_the_next_scene_leave_before_the_backdrop_changes_and_stayers_stay() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 1, 2]);
        harness.run_to(500);
        harness.view(scene(None, Vec::new(), vec![0, 1, 3]));
        harness.run_to(8_000);
        let render = harness.rendered()[1];
        let exit_of = |slot: usize| {
            harness.first_time(|command| {
                matches!(command, Command::Actor { slot: s, state: ActorState::Exiting } if *s == slot)
            })
        };
        assert!(exit_of(2).expect("slot 2 leaves") <= render);
        assert_eq!(exit_of(0), None);
        assert_eq!(exit_of(1), None);
        assert_eq!(harness.conductor.actor_state(0), ActorState::Idle);
        assert_eq!(harness.conductor.actor_state(2), ActorState::Offstage);
        assert_eq!(harness.conductor.actor_state(3), ActorState::Idle);
    }

    #[test]
    fn the_speaking_mark_follows_the_line_not_the_view() {
        let mut harness = Harness::new(Voice::working(2_000), true, Vec::new(), vec![0, 2]);
        harness.view(plain(
            vec![
                line("narration", true, None),
                line("courier", true, Some(2)),
            ],
            vec![0, 2],
        ));
        harness.run_to(300);
        assert_eq!(harness.conductor.actor_state(2), ActorState::Idle);
        // The narration opens and speaks for 2 s, lingers and closes; the courier then opens.
        let courier_opens = u64::from(LINE_OPEN_MS + 2_000 + LINE_LINGER_MS + LINE_CLOSE_MS);
        harness.run_to(courier_opens + u64::from(LINE_OPEN_MS) + 1_000);
        assert_eq!(harness.conductor.actor_state(2), ActorState::Speaking);
        harness.run_to(
            courier_opens + 2 * u64::from(LINE_OPEN_MS) + 2_000 + u64::from(LINE_LINGER_MS) + 2_000,
        );
        assert_eq!(harness.conductor.actor_state(2), ActorState::Idle);
    }

    #[test]
    fn a_restart_shows_a_settled_line_at_once_and_waits_for_a_choice() {
        let mut conductor = TestConductor::new();
        let mut settled = line("answer", false, Some(2));
        settled.settled = true;
        let commands = conductor.restart(0, plain(vec![settled], vec![0, 2]));
        assert!(commands.iter().any(|command| matches!(
            command,
            Command::Line {
                state: LineState::Lingering,
                detail: "answer",
                ..
            }
        )));
        assert_eq!(conductor.scene_state(), SceneState::AwaitingChoice);
        assert_eq!(conductor.actor_state(2), ActorState::Idle);
    }

    #[test]
    fn signals_for_unknown_lines_and_ticks_before_any_view_change_nothing() {
        let mut conductor = TestConductor::new();
        assert!(conductor.on_tick(10).is_empty());
        assert!(conductor.on_clip(20, 99, ClipSignal::Finished).is_empty());
        assert_eq!(conductor.scene_state(), SceneState::Hidden);
        assert_eq!(conductor.focus_line(), None);
    }

    #[test]
    fn a_scene_that_has_said_everything_exits_with_its_bubbles_closed_first() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.view(scene(None, vec![line("only", true, None)], vec![0]));
        harness.run_to(15_000);
        harness.view(scene(None, vec![line("next", true, None)], vec![0]));
        harness.run_to(30_000);
        assert!(harness.opened("next").is_some());
        assert_eq!(harness.conductor.director_state(), StageState::Idle);
        assert!(harness.closing("only").expect("only closes") < harness.rendered()[2]);
        let _ = LINE_CLOSE_MS;
    }

    #[test]
    fn a_clip_error_ends_the_line_on_reading_time_and_the_conductor_goes_quiet() {
        let voice = Voice {
            length_ms: None,
            ..Voice::working(30_000)
        };
        let mut harness = Harness::new(voice, true, Vec::new(), vec![0]);
        harness.view(scene(None, vec![line("narration", true, None)], vec![0]));
        harness.run_to(8_000);
        assert_eq!(harness.focus(), Some(LineState::Speaking));
        assert!(harness.conductor.is_busy());
        let id = harness.conductor.focus_line().expect("a line").0;
        let commands = harness
            .conductor
            .on_clip(harness.now, id, ClipSignal::Error);
        assert!(commands.iter().any(|command| matches!(
            command,
            Command::Line {
                state: LineState::Lingering,
                ..
            }
        )));
        assert!(!harness.conductor.lines_waiting());
        assert_eq!(harness.conductor.active_speaker(), Some(None));
        harness.run_to(60_000);
        // The final line lingers with nothing behind it: only an event wakes the conductor.
        assert!(!harness.conductor.is_busy());
        assert_eq!(harness.conductor.scene_state(), SceneState::AwaitingChoice);
    }

    fn fight_start() -> ViewPlan<&'static str> {
        ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                enters_combat: true,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        }
    }

    fn first_attack() -> ViewPlan<&'static str> {
        ViewPlan {
            diff: ViewDiff {
                roll: Some("enemy attack"),
                impact: Some("enemy attack"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        }
    }

    fn battle_states(harness: &Harness) -> Vec<BattleState> {
        harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Battle(state) => Some(*state),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_absent_battle_never_owns_the_scene_even_with_webgpu_and_a_pending_renderer() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        for renderer in [
            Battlefield::Pending,
            Battlefield::Ready,
            Battlefield::Fallback,
        ] {
            let commands = harness.conductor.on_battlefield(harness.now, renderer);
            harness.record(commands);
            assert_eq!(harness.conductor.battle_state(), BattleState::Absent);
            assert!(!harness.conductor.battlefield_live());
        }
        harness.view(ViewPlan {
            diff: ViewDiff::default(),
            script: vec![line("dialogue", false, Some(0))],
            cast: vec![0],
        });
        harness.run_to(5_000);
        assert!(harness.opened("dialogue").is_some());
        assert!(!harness.conductor.battlefield_live());
    }

    #[test]
    fn a_battle_owns_the_stage_until_exit_and_resets_renderer_readiness_on_reentry() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.run_to(5_000);
        assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
        assert!(harness.conductor.battlefield_live());

        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        assert_eq!(harness.conductor.battle_state(), BattleState::TurnStart);
        assert!(harness.conductor.battlefield_live());

        harness.view(ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                roll: Some("blow"),
                impact: Some("blow"),
                defeat: Some("blow"),
                banner: Some("victory"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        });
        harness.run_to(30_000);
        assert!(battle_states(&harness).contains(&BattleState::Exiting));
        assert_eq!(harness.conductor.battle_state(), BattleState::Absent);
        assert!(!harness.conductor.battlefield_live());

        // The old renderer can be Ready or Fallback after exit; neither belongs to this fight.
        for old_renderer in [Battlefield::Ready, Battlefield::Fallback] {
            let commands = harness.conductor.on_battlefield(harness.now, old_renderer);
            harness.record(commands);
            harness.view(fight_start());
            harness.run_to(harness.now + 5_000);
            assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
            assert!(harness.conductor.battlefield_live());
            let commands = harness.conductor.restart(
                harness.now,
                ViewPlan {
                    diff: ViewDiff::default(),
                    script: Vec::new(),
                    cast: vec![0],
                },
            );
            harness.record(commands);
            assert_eq!(harness.conductor.battle_state(), BattleState::Absent);
            assert!(!harness.conductor.battlefield_live());
        }
    }

    #[test]
    fn the_first_attack_waits_for_the_battlefield_entrance_and_the_flat_view_is_not_released_early()
    {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.run_to(300);
        // The enemy's first attack arrives while the scene is still changing.
        harness.view(first_attack());
        harness.run_to(6_000);
        assert_eq!(harness.conductor.scene_state(), SceneState::AwaitingChoice);
        assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
        assert!(harness.conductor.battlefield_live());
        assert!(
            harness.entered(StageState::Rolling).is_none(),
            "no attack before the entrance"
        );
        assert_eq!(battle_states(&harness), [BattleState::Entering]);
        // The battlefield finishes its banner, initiative roll and roll-call.
        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 3_000);
        let rolling = harness
            .entered(StageState::Rolling)
            .expect("the attack now plays");
        assert!(rolling >= 6_000);
        assert_eq!(
            battle_states(&harness),
            [
                BattleState::Entering,
                BattleState::TurnStart,
                BattleState::Acting,
                BattleState::Resolving,
                BattleState::TurnEnd,
            ]
        );
    }

    #[test]
    fn fight_introduction_is_voiced_during_the_entrance_before_the_first_held_attack() {
        let mut harness = Harness::new(Voice::working(1_000), true, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        let mut start = fight_start();
        start.script = vec![line("fight introduction", true, None)];
        harness.view(start);
        harness.view(first_attack());

        harness.run_to(6_000);

        let intro_opened = harness.opened("fight introduction").expect("intro opens");
        let intro_voiced = harness
            .play_at("fight introduction")
            .expect("intro voice starts");
        assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
        assert!(harness.entered(StageState::Rolling).is_none());

        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 5_000);

        let first_attack = harness
            .entered(StageState::Rolling)
            .expect("held attack releases");
        let intro_finished = harness
            .line_at("fight introduction", LineState::Lingering)
            .expect("intro voice finishes");
        assert!(intro_opened < first_attack);
        assert!(intro_voiced < first_attack);
        assert!(intro_finished < first_attack);
    }

    #[test]
    fn a_held_opening_beat_counts_as_busy_until_the_entrance_finishes() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.run_to(300);
        harness.view(first_attack());
        harness.run_to(6_000);
        assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
        assert_eq!(harness.conductor.presented_sequence(), 0);
        assert!(
            !harness.conductor.director_idle(),
            "a beat held for the entrance must not read as idle: the display would \
             otherwise report it presented before the field is raised"
        );
        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 10_000);
        assert!(harness.conductor.director_idle());
    }

    #[test]
    fn a_progressing_battlefield_keeps_opening_blows_held_until_it_is_ready() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        for turn in ["first enemy", "second enemy", "third enemy"] {
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some(turn),
                    impact: Some(turn),
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: vec![0],
            });
        }

        for second in 1_u64..=40 {
            let now = second * 1_000;
            harness.conductor.on_battlefield_progress(now);
            let commands = harness.conductor.on_battlefield(now, Battlefield::Pending);
            harness.record(commands);
            harness.run_to(now);
            assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
            assert!(!harness.conductor.director_idle());
            assert_eq!(harness.conductor.presented_sequence(), 0);
            assert!(harness.entered(StageState::Rolling).is_none());
        }

        harness.now += 1_000;
        let ready_at = harness.now;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(ready_at + 20_000);
        let rolls: Vec<&str> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state: StageState::Rolling,
                    detail,
                    ..
                } => *detail,
                _ => None,
            })
            .collect();
        assert_eq!(rolls, ["first enemy", "second enemy", "third enemy"]);
    }

    #[test]
    fn without_a_battlefield_the_entrance_ends_with_the_scene_and_a_fallback_releases_it() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.view(fight_start());
        harness.view(first_attack());
        harness.run_to(5_000);
        assert!(harness.entered(StageState::Rolling).is_some());
        assert!(!harness.conductor.battlefield_live());

        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.view(first_attack());
        harness.run_to(3_000);
        assert!(harness.entered(StageState::Rolling).is_none());
        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Fallback);
        harness.record(commands);
        harness.run_to(harness.now + 2_000);
        assert!(harness.entered(StageState::Rolling).is_some());
        assert!(!harness.conductor.battlefield_live());
    }

    #[test]
    fn a_battlefield_that_never_reports_is_given_up_on_at_the_cap() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.view(first_attack());
        harness.run_to(BATTLE_ENTER_CAP_MS + 8_000);
        let rolling = harness
            .entered(StageState::Rolling)
            .expect("released at the cap");
        assert!(rolling >= BATTLE_ENTER_CAP_MS);
        assert_ne!(harness.conductor.battle_state(), BattleState::Entering);
        assert!(harness.conductor.battlefield_failed());
        assert!(!harness.conductor.battlefield_live());
    }

    #[test]
    fn the_finishing_blow_leads_through_finishing_and_exiting_back_to_absent() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0]);
        harness.view(fight_start());
        harness.run_to(5_000);
        harness.view(ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                roll: Some("blow"),
                impact: Some("blow"),
                defeat: Some("blow"),
                banner: Some("victory"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        });
        harness.run_to(30_000);
        let seen = battle_states(&harness);
        for expected in [
            BattleState::Acting,
            BattleState::Resolving,
            BattleState::Finishing,
            BattleState::Exiting,
            BattleState::Absent,
        ] {
            assert!(seen.contains(&expected), "{expected:?} in {seen:?}");
        }
        assert_eq!(harness.conductor.battle_state(), BattleState::Absent);
    }

    fn mark(slot: usize, kind: IntentKind) -> IntentMark<&'static str> {
        IntentMark {
            slot,
            kind,
            detail: match (slot, kind) {
                (0, IntentKind::Typing) => "typing 0",
                (0, IntentKind::Speaking) => "speaking 0",
                (0, IntentKind::Submitted) => "submitted 0",
                (_, IntentKind::Typing) => "typing 1",
                (_, IntentKind::Speaking) => "speaking 1",
                (_, IntentKind::Submitted) => "submitted 1",
            },
        }
    }

    impl Harness {
        fn intents(&mut self, marks: Vec<IntentMark<&'static str>>) {
            let commands = self.conductor.on_intents(self.now, marks);
            self.record(commands);
        }

        /// The line states seen for a payload, in order.
        fn line_trace(&self, detail: &'static str) -> Vec<LineState> {
            self.log
                .iter()
                .filter_map(|(_, command)| match command {
                    Command::Line {
                        state, detail: d, ..
                    } if *d == detail => Some(*state),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn a_loader_follows_typing_to_submitted_and_the_heros_line_takes_its_place_in_order() {
        let mut harness = Harness::new(Voice::working(2_000), true, Vec::new(), vec![0, 1, 2]);
        harness.intents(vec![mark(0, IntentKind::Typing)]);
        harness.run_to(500);
        harness.intents(vec![mark(0, IntentKind::Typing)]);
        harness.intents(vec![mark(0, IntentKind::Submitted)]);
        assert_eq!(harness.line_trace("typing 0"), [LineState::Composing]);
        // The line is committed: the same view no longer carries the loader.
        harness.view(plain(
            vec![line("hero line", false, Some(0))],
            vec![0, 1, 2],
        ));
        harness.intents(Vec::new());
        harness.run_to(1_500);
        let opened = harness.opened("hero line").expect("the line opens");
        let replaced = harness
            .line_at("submitted 0", LineState::Done)
            .expect("the loader is replaced");
        assert!(replaced >= opened);
        assert!(harness.line_at("typing 0", LineState::Cancelled).is_none());
        harness.view(plain(vec![line("npc reply", true, Some(2))], vec![0, 1, 2]));
        harness.run_to(20_000);
        assert!(harness.opened("npc reply").expect("reply") > opened);
    }

    #[test]
    fn two_heroes_composing_at_once_each_get_their_own_loader_and_never_delay_speech() {
        let mut harness = Harness::new(
            Voice::working(3_000),
            true,
            vec![line("narration", true, None)],
            vec![0, 1, 2],
        );
        harness.intents(vec![
            mark(0, IntentKind::Typing),
            mark(1, IntentKind::Speaking),
        ]);
        harness.run_to(1_000);
        assert_eq!(harness.line_trace("typing 0"), [LineState::Composing]);
        assert_eq!(harness.line_trace("speaking 1"), [LineState::Composing]);
        let narration = harness
            .opened("narration")
            .expect("narration opens regardless");
        // The narration's clip started at its own time, unaffected by the loaders (the harness
        // also asserts that no two lines are ever active together).
        assert!(narration <= 200, "{narration}");
    }

    #[test]
    fn a_hero_who_stops_cancels_the_loader_through_the_normal_exit() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 1]);
        harness.intents(vec![mark(0, IntentKind::Typing)]);
        harness.run_to(300);
        harness.intents(Vec::new());
        harness.run_to(1_000);
        assert_eq!(
            harness.line_trace("typing 0"),
            [
                LineState::Composing,
                LineState::Cancelled,
                LineState::Closing,
                LineState::Done
            ]
        );
    }

    #[test]
    fn an_unrefreshed_loader_expires_and_a_refreshed_one_stays() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 1]);
        harness.intents(vec![mark(0, IntentKind::Typing)]);
        for _ in 0..4 {
            harness.run_to(harness.now + 2_000);
            harness.intents(vec![mark(0, IntentKind::Typing)]);
        }
        // Eight seconds of refreshes: still there.
        assert_eq!(harness.line_trace("typing 0"), [LineState::Composing]);
        let last = harness.now;
        // The tab closes: no more refreshes.
        harness.run_to(last + INTENT_EXPIRY_MS + 300);
        let closing = harness
            .line_at("typing 0", LineState::Closing)
            .expect("expired");
        assert!(closing >= last + INTENT_EXPIRY_MS);
        assert!(closing <= last + INTENT_EXPIRY_MS + 200);
        harness.run_to(last + INTENT_EXPIRY_MS + 1_000);
        assert!(harness.line_at("typing 0", LineState::Done).is_some());
    }

    #[test]
    fn a_submitted_loader_waits_for_its_line_queued_behind_speech() {
        let mut harness = Harness::new(
            Voice::working(6_000),
            true,
            vec![line("narration", true, None)],
            vec![0, 1],
        );
        harness.run_to(500);
        harness.intents(vec![mark(0, IntentKind::Submitted)]);
        harness.view(plain(vec![line("hero line", false, Some(0))], vec![0, 1]));
        harness.intents(Vec::new());
        // Well past the loader's own expiry, the line is still behind the narration.
        harness.run_to(5_900);
        assert!(harness.opened("hero line").is_none());
        assert!(harness.line_at("submitted 0", LineState::Done).is_none());
        assert!(harness.line_at("submitted 0", LineState::Closing).is_none());
        harness.run_to(15_000);
        assert!(harness.opened("hero line").is_some());
        assert!(harness.line_at("submitted 0", LineState::Done).is_some());
    }

    #[test]
    fn a_loader_leaves_with_the_scene() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 1]);
        harness.intents(vec![mark(0, IntentKind::Typing)]);
        harness.view(scene(None, Vec::new(), vec![0, 1]));
        harness.run_to(4_000);
        assert!(harness.line_at("typing 0", LineState::Closing).is_some());
    }

    fn killing_blow_plan() -> ViewPlan<&'static str> {
        ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                roll: Some("blow"),
                impact: Some("blow"),
                defeat: Some("blow"),
                banner: Some("victory"),
                closing: Some("card"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0],
        }
    }

    /// A fight that ends on its very first action: the entrance completes, then the one view that
    /// carries the kill arrives.
    fn first_action_kill() -> Harness {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        harness.run_to(2_000);
        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 200);
        assert_eq!(harness.conductor.battle_state(), BattleState::TurnStart);
        harness.view(killing_blow_plan());
        harness
    }

    #[test]
    fn a_fight_that_ends_on_its_first_action_plays_the_whole_finishing_sequence_before_the_victory_scene()
     {
        let mut harness = first_action_kill();
        let kill_at = harness.now;
        let rendered_before = harness.rendered().len();
        // The blow's cues play; the battlefield is busy with its finishing shot.
        let mut battlefield_busy_until = harness.now + 6_000;
        while harness.now < harness.now.max(battlefield_busy_until) + 6_000 {
            harness.run_to(harness.now + 50);
            let state = if harness.now < battlefield_busy_until {
                Battlefield::Pending
            } else {
                Battlefield::Ready
            };
            let commands = harness.conductor.on_battlefield(harness.now, state);
            harness.record(commands);
            if harness
                .entered_after(StageState::Aftermath, kill_at)
                .is_some()
            {
                break;
            }
        }
        let rolling = harness
            .entered_after(StageState::Rolling, kill_at)
            .expect("dice");
        let impact = harness
            .entered_after(StageState::Impact, kill_at)
            .expect("impact");
        let defeat = harness
            .entered_after(StageState::Defeat, kill_at)
            .expect("fall");
        let leaving = harness
            .entered_after(StageState::LeavingScene, kill_at)
            .expect("leaves");
        let aftermath = harness
            .entered_after(StageState::Aftermath, kill_at)
            .expect("banner");
        assert!(rolling < impact && impact < defeat && defeat < leaving && leaving < aftermath);
        // The victory scene waits for the battlefield's own finishing shot.
        assert!(
            leaving >= battlefield_busy_until,
            "{leaving} before {battlefield_busy_until}"
        );
        // No redraw (so no teardown of the battlefield) before the exit.
        assert_eq!(
            harness
                .rendered()
                .iter()
                .filter(|at| **at < leaving)
                .count(),
            rendered_before
        );
        let seen = battle_states(&harness);
        let order: Vec<BattleState> = seen
            .iter()
            .copied()
            .skip_while(|state| *state != BattleState::Acting)
            .collect();
        assert_eq!(
            order,
            [
                BattleState::Acting,
                BattleState::Resolving,
                BattleState::Finishing,
                BattleState::Exiting,
                BattleState::Absent,
            ]
        );
        battlefield_busy_until = 0;
        let _ = battlefield_busy_until;
    }

    #[test]
    fn a_battlefield_that_never_finishes_its_shot_holds_the_exit_only_to_the_cap() {
        let mut harness = first_action_kill();
        let start = harness.now;
        let kill_at = start;
        while harness.now < start + 30_000 {
            harness.run_to(harness.now + 50);
            let commands = harness
                .conductor
                .on_battlefield(harness.now, Battlefield::Pending);
            harness.record(commands);
        }
        let leaving = harness
            .entered_after(StageState::LeavingScene, kill_at)
            .expect("released at the cap");
        // The blow's cues (about 4.3 s) plus the cap bound the wait.
        assert!(
            leaving <= start + 4_500 + BATTLE_FINISH_CAP_MS + 500,
            "{}",
            leaving - start
        );
        assert!(
            leaving >= start + BATTLE_FINISH_CAP_MS,
            "{} {}",
            leaving,
            start
        );
    }

    #[test]
    fn without_a_battlefield_the_victory_follows_the_blow_at_once() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.view(fight_start());
        harness.run_to(4_000);
        harness.view(killing_blow_plan());
        let kill_at = harness.now;
        harness.run_to(harness.now + 9_000);
        let defeat = harness
            .entered_after(StageState::Defeat, kill_at)
            .expect("fall");
        let leaving = harness
            .entered_after(StageState::LeavingScene, kill_at)
            .expect("leaves");
        assert!(leaving - defeat <= 3_000);
    }

    #[test]
    fn an_enemy_that_won_initiative_has_its_opening_turn_staged_after_the_whole_entrance() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.conductor.set_battlefield_expected(true);
        // One view starts the fight and already carries the goblin's resolved first attack.
        harness.view(ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                enters_combat: true,
                roll: Some("goblin attack"),
                impact: Some("goblin attack"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        });
        harness.run_to(8_000);
        assert!(harness.conductor.wants_battlefield());
        // The scene changed behind the cover, the battlefield is still loading: no dice, no impact.
        assert!(harness.entered(StageState::Rolling).is_none());
        assert!(harness.entered(StageState::Impact).is_none());
        assert_eq!(harness.conductor.battle_state(), BattleState::Entering);
        // Exactly one transition into the fight.
        let leavings = harness
            .log
            .iter()
            .filter(|(_, command)| {
                matches!(
                    command,
                    Command::Enter {
                        state: StageState::LeavingScene,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(leavings, 1);
        // The battlefield finishes its splat load, banner, initiative roll and roll-call.
        harness.now += 50;
        let ready_at = harness.now;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 4_000);
        let rolling = harness
            .entered(StageState::Rolling)
            .expect("the goblin's turn plays");
        assert!(rolling >= ready_at);
        assert!(harness.entered(StageState::Impact).is_some());
    }

    #[test]
    fn a_display_that_comes_up_in_a_fight_still_enters_it_behind_the_battlefield() {
        let mut conductor = TestConductor::new();
        conductor.set_battlefield_expected(true);
        let commands = conductor.restart(
            0,
            ViewPlan {
                diff: ViewDiff {
                    enters_combat: true,
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: vec![0, 2],
            },
        );
        assert!(commands.contains(&Command::Battle(BattleState::Entering)));
        assert_eq!(conductor.battle_state(), BattleState::Entering);
        assert!(conductor.wants_battlefield());
        let released = conductor.on_battlefield(100, Battlefield::Ready);
        assert!(released.contains(&Command::Battle(BattleState::TurnStart)));
    }

    #[test]
    fn several_outcomes_in_one_view_play_in_order_after_the_entrance() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.conductor.set_battlefield_expected(true);
        harness.view(fight_start());
        for (index, turn) in ["first enemy", "second enemy", "third enemy"]
            .into_iter()
            .enumerate()
        {
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some(turn),
                    impact: Some(turn),
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: vec![0, 2],
            });
            assert_eq!(
                harness.conductor.battle_state(),
                BattleState::Entering,
                "{index}"
            );
        }
        harness.run_to(2_000);
        assert!(harness.entered(StageState::Rolling).is_none());
        harness.now += 50;
        let commands = harness
            .conductor
            .on_battlefield(harness.now, Battlefield::Ready);
        harness.record(commands);
        harness.run_to(harness.now + 12_000);
        let rolls: Vec<&str> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state: StageState::Rolling,
                    detail,
                    ..
                } => *detail,
                _ => None,
            })
            .collect();
        assert_eq!(rolls, ["first enemy", "second enemy", "third enemy"]);
    }

    #[test]
    fn a_fight_lost_on_an_enemys_turn_shows_every_blow_then_exits_through_the_same_machine() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.view(fight_start());
        harness.run_to(5_000);
        let rendered_before = harness.rendered().len();
        let blow = |turn: &'static str| ViewPlan {
            diff: ViewDiff {
                roll: Some(turn),
                impact: Some(turn),
                blow_only: true,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        };
        harness.view(blow("first enemy"));
        harness.view(blow("second enemy"));
        harness.view(ViewPlan {
            diff: ViewDiff {
                scene_changed: true,
                roll: Some("last enemy"),
                impact: Some("last enemy"),
                defeat: Some("last enemy"),
                banner: Some("defeat"),
                closing: Some("closing"),
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: vec![0, 2],
        });
        let lost_at = harness.now;
        harness.run_to(40_000);
        let impacts: Vec<&str> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state: StageState::Impact,
                    detail,
                    ..
                } => *detail,
                _ => None,
            })
            .collect();
        assert_eq!(impacts, ["first enemy", "second enemy", "last enemy"]);
        let leaving = harness
            .entered_after(StageState::LeavingScene, lost_at)
            .expect("the scene leaves");
        // The earlier blows never redrew the view, which would tear the battlefield down.
        assert_eq!(
            harness
                .rendered()
                .iter()
                .filter(|at| **at < leaving)
                .count(),
            rendered_before
        );
        let order: Vec<BattleState> = battle_states(&harness)
            .into_iter()
            .filter(|state| {
                matches!(
                    state,
                    BattleState::Finishing | BattleState::Exiting | BattleState::Absent
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                BattleState::Finishing,
                BattleState::Exiting,
                BattleState::Absent
            ]
        );
        assert_eq!(harness.conductor.battle_state(), BattleState::Absent);
    }

    #[test]
    fn three_enemy_turns_resolved_at_once_each_show_their_blow_and_the_turn_closes() {
        let mut harness = Harness::new(Voice::working(1_000), false, Vec::new(), vec![0, 2]);
        harness.view(fight_start());
        harness.run_to(6_000);
        assert_eq!(harness.conductor.battle_state(), BattleState::TurnStart);
        // The hero's blow and three enemy turns arrive in one burst; the display is slow.
        for turn in ["hero", "first enemy", "second enemy", "third enemy"] {
            harness.view(ViewPlan {
                diff: ViewDiff {
                    roll: Some(turn),
                    impact: Some(turn),
                    turn_changed: turn == "third enemy",
                    ..ViewDiff::default()
                },
                script: Vec::new(),
                cast: vec![0, 2],
            });
        }
        harness.run_to(harness.now + 20_000);
        let impacts: Vec<&str> = harness
            .log
            .iter()
            .filter_map(|(_, command)| match command {
                Command::Enter {
                    state: StageState::Impact,
                    detail,
                    ..
                } => *detail,
                _ => None,
            })
            .collect();
        assert_eq!(
            impacts,
            ["hero", "first enemy", "second enemy", "third enemy"]
        );
        assert_eq!(harness.conductor.battle_state(), BattleState::TurnStart);
    }
}
