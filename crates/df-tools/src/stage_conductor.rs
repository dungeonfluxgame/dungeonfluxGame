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
//!   change waits, the remaining lines of the scene on screen are shown briefly as text and the
//!   display converges on the newest scene; no wait exceeds [`SCENE_WAIT_CAP_MS`].
//! - Outcomes (dice, a defeat, a fight's start or end) are never held behind story speech: the
//!   line in progress fades within [`FIGHT_LINE_FADE_MS`] and the director plays the outcome.
//!
//! There is no clock inside: callers pass `now_ms`, so tests use a controlled one. The
//! conductor decides nothing about the game and sends nothing; the newest accepted view is
//! always what a `Render` command draws.
use crate::{
    stage_director::{Command as DirectorCommand, StageDirector, StageState, ViewDiff},
    stage_machines::{
        ACTOR_ENTER_MS, ACTOR_EXIT_MS, ActorEvent, ActorState, CATCH_UP_TEXT_MS, CLIP_END_GRACE_MS,
        CLIP_READY_WAIT_MS, CLIP_START_WAIT_MS, ClipEvent, ClipState, FIGHT_LINE_FADE_MS,
        FINAL_LINGER_CAP_MS, LINE_CLOSE_MS, LINE_LINGER_MS, LINE_OPEN_MS, LineEvent, LineState,
        SCENE_ENTER_CAP_MS, SCENE_EXIT_CAP_MS, SCENE_WAIT_CAP_MS, SceneEvent, SceneState,
        actor_next, clip_next, line_next, scene_next,
    },
};

pub(crate) type LineId = u64;

/// One line a view puts on the stage. The payload is opaque to the conductor.
#[derive(Clone, Debug)]
pub(crate) struct ScriptLine<D> {
    pub(crate) detail: D,
    /// A clip can be played for it (narration and non-player speech); a hero's typed line and
    /// an action call-out are silent.
    pub(crate) voiceable: bool,
    /// How long it is read when it has no sound.
    pub(crate) read_ms: u32,
    /// The actor slot whose figure carries the bubble; `None` for the narrator's box.
    pub(crate) slot: Option<usize>,
    /// Already on screen before the page was (reload, reconnect): it stands Lingering,
    /// silently, with no entrance.
    pub(crate) settled: bool,
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
    /// Shown briefly as text because the display is catching up.
    catch_up: bool,
    close_ms: u32,
    /// The line began with its clip playing.
    voiced: bool,
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
}

impl<D: Clone> Default for Conductor<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Clone> Conductor<D> {
    pub(crate) fn new() -> Self {
        Self {
            scene: SceneState::Hidden,
            scene_since_ms: 0,
            director: StageDirector::new(),
            lines: Vec::new(),
            pending: None,
            actors: Vec::new(),
            next_id: 1,
            sound_on: false,
            in_entrance: false,
        }
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
            LineState::Done => 3,
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
        self.director.state() != StageState::Idle
            || matches!(self.scene, SceneState::Entering | SceneState::Exiting)
            || self.pending.is_some()
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
        for run in std::mem::take(&mut self.lines) {
            self.cancel_clip(&run, &mut out);
        }
        self.pending = None;
        self.in_entrance = false;
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
        if outcome {
            self.fade_lines_for_outcome(now_ms, &mut out);
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
            let commands = self.director.on_view(now_ms, plan.diff);
            self.absorb(now_ms, commands, &mut out);
        } else if plan.diff.scene_changed {
            self.set_pending(now_ms, plan);
        } else if let Some(pending) = self.pending.as_mut() {
            // The scene on screen has not left yet: what this view adds belongs to its
            // successor, behind the lines already waiting there.
            pending.script.extend(plan.script);
            pending.cast = plan.cast;
        } else {
            self.install_more(plan.script, now_ms, &mut out);
            let commands = self.director.on_view(now_ms, plan.diff);
            self.absorb(now_ms, commands, &mut out);
        }
        self.step(now_ms, &mut out);
        out
    }

    pub(crate) fn on_tick(&mut self, now_ms: u64) -> Vec<Command<D>> {
        let mut out = Vec::new();
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
            }
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
                DirectorCommand::Rest => {
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
        LineRun {
            id,
            line,
            state,
            clip: ClipState::Silent,
            since_ms: now_ms,
            clip_since_ms: now_ms,
            clip_limit_ms: 0,
            play_sent: false,
            catch_up: false,
            close_ms: LINE_CLOSE_MS,
            voiced: false,
        }
    }

    /// Replaces the scene's lines with a new scene's script.
    fn install_script(
        &mut self,
        script: Vec<ScriptLine<D>>,
        now_ms: u64,
        out: &mut Vec<Command<D>>,
    ) {
        for run in std::mem::take(&mut self.lines) {
            self.cancel_clip(&run, out);
        }
        self.install_more(script, now_ms, out);
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
        let run = run.clone();
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
        // Several quick changes, or a wait past the cap, close the line in progress.
        let catching_up = self.catching_up(now_ms);
        let overdue = self
            .pending
            .as_ref()
            .is_some_and(|pending| now_ms.saturating_sub(pending.since_ms) > SCENE_WAIT_CAP_MS);
        for index in 0..self.lines.len() {
            self.advance_line(index, now_ms, catching_up, overdue, out);
        }
        self.start_next_line(now_ms, catching_up, out);
        self.prefetch(out);
        self.settle_scene(now_ms, out);
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
        overdue: bool,
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
                self.watch_speech(index, now_ms, overdue, out);
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

    /// Ends the speech of an Opening or Speaking line when its clip is over, its reading time
    /// has passed, or its clip's end signal is overdue; a scene waiting too long closes it.
    fn watch_speech(
        &mut self,
        index: usize,
        now_ms: u64,
        overdue: bool,
        out: &mut Vec<Command<D>>,
    ) {
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
        let spoken = if run.voiced {
            run.clip.is_over()
        } else {
            let hold = if run.catch_up {
                CATCH_UP_TEXT_MS
            } else {
                run.line.read_ms
            };
            run.state == LineState::Speaking
                && now_ms.saturating_sub(run.since_ms) >= u64::from(hold)
        };
        if overdue {
            let snapshot = run.clone();
            self.cancel_clip(&snapshot, out);
            if let Some(run) = self.lines.get_mut(index)
                && let Some(next) = clip_next(run.clip, ClipEvent::Cancel)
            {
                run.clip = next;
            }
            self.set_line(index, LineEvent::Close, now_ms, out);
        } else if spoken {
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
        if self.scene == SceneState::AwaitingChoice {
            self.move_scene(now_ms, SceneEvent::LinesResumed, out);
        }
        let Some(run) = self.lines.get_mut(index) else {
            return;
        };
        let wants_sound = run.line.voiceable && self.sound_on && !catching_up;
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
        run.catch_up = catching_up;
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
        self.set_line(index, LineEvent::Begin, now_ms, out);
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
        CATCH_UP_TEXT_MS, CLIP_READY_WAIT_MS, CLIP_START_WAIT_MS, LINE_CLOSE_MS,
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

        fn entered(&self, wanted: StageState) -> Option<u64> {
            self.first_time(
                |command| matches!(command, Command::Enter { state, .. } if *state == wanted),
            )
        }

        fn focus(&self) -> Option<LineState> {
            self.conductor.focus_line().map(|(_, state)| state)
        }
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
        // The old narration finished (its clip was never cut), the old reply was shown
        // briefly as text only, and the newest scene opens within the stated bound.
        let narration_end = harness
            .line_at("old narration", LineState::Lingering)
            .expect("old narration ends");
        assert!(
            narration_end >= 6_000,
            "the old clip is not cut: {narration_end}"
        );
        let reply_open = harness.opened("old reply").expect("shown briefly");
        let reply_over = harness
            .line_at("old reply", LineState::Lingering)
            .expect("old reply ends");
        assert!(
            reply_over - reply_open <= u64::from(CATCH_UP_TEXT_MS) + u64::from(LINE_OPEN_MS) + 100
        );
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
        harness.run_to(15_000);
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
    fn a_waiting_scene_never_waits_longer_than_the_cap() {
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
        let swap = harness.rendered()[1];
        assert!(swap >= waiting_since + SCENE_WAIT_CAP_MS);
        assert!(swap <= waiting_since + SCENE_WAIT_CAP_MS + 1_500, "{swap}");
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
        harness.run_to(4_500);
        assert_eq!(harness.conductor.actor_state(2), ActorState::Speaking);
        harness.run_to(9_000);
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
}
