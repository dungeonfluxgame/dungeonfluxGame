//! The display's presentation state machine: WHEN each change between views is shown.
//!
//! `presentation` works out what changed. This director orders those changes into a FIFO of
//! cues and plays them one at a time, so a killing blow is seen before the scene moves on, a
//! scene change always leaves, shows the interstitial and enters before its narration, and only
//! one voice speaks at once. It is pure and presentation-only: it decides no outcome, sends
//! nothing, and the newest accepted view is always what its `Render` command draws. There is no
//! clock inside; callers pass `now_ms`, so tests use a controlled one.
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
const VOICE_MINIMUM_MS: u32 = 1_200;
const VOICE_GRACE_MS: u32 = 3_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StageState {
    Idle,
    LeavingScene,
    /// The wordmark shows while the painting changes underneath.
    Interstitial,
    EnteringScene,
    Narrating,
    Speaking,
    Rolling,
    Impact,
    Defeat,
    Healing,
    Aftermath,
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
    Narrate,
    Speak,
    Banner,
}

#[derive(Clone, Debug)]
pub(crate) struct Cue<D> {
    kind: CueKind,
    detail: Option<D>,
    hold_ms: u32,
    voiced: bool,
    /// Part of the ending of a fight: kept, shortened, when the display falls behind.
    outcome: bool,
    short: bool,
    queued_ms: u64,
}

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
    pub(crate) roll: Option<D>,
    pub(crate) impact: Option<D>,
    /// The target of the roll dropped to zero hit points.
    pub(crate) defeat: Option<D>,
    pub(crate) heal: Option<D>,
    /// The fight ended in this view; the payload describes the banner.
    pub(crate) banner: Option<D>,
    pub(crate) narration: Option<(D, u32)>,
    pub(crate) speech: Option<(D, u32)>,
    /// A voice clip is expected, so a line waits for its end signal (up to a bound).
    pub(crate) voiced: bool,
}

impl<D> Default for ViewDiff<D> {
    fn default() -> Self {
        Self {
            scene_changed: false,
            roll: None,
            impact: None,
            defeat: None,
            heal: None,
            banner: None,
            narration: None,
            speech: None,
            voiced: false,
        }
    }
}

#[derive(Clone, Debug)]
struct Active<D> {
    kind: CueKind,
    state: StageState,
    detail: Option<D>,
    started_ms: u64,
    minimum_ms: u32,
    maximum_ms: u32,
    waits_for_signal: bool,
    signalled: bool,
    short: bool,
    voiced: bool,
}

pub(crate) struct StageDirector<D> {
    queue: VecDeque<Cue<D>>,
    active: Option<Active<D>>,
}

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
        }
    }

    pub(crate) fn state(&self) -> StageState {
        self.active
            .as_ref()
            .map_or(StageState::Idle, |active| active.state)
    }

    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Starts again at the current view with nothing queued: first view, reload, reconnect.
    pub(crate) fn restart(&mut self) -> Vec<Command<D>> {
        self.queue.clear();
        self.active = None;
        vec![Command::Render]
    }

    fn cue(kind: CueKind, detail: Option<D>, now_ms: u64, outcome: bool) -> Cue<D> {
        Cue {
            kind,
            detail,
            hold_ms: 0,
            voiced: false,
            outcome,
            short: false,
            queued_ms: now_ms,
        }
    }

    /// Queues the cues of a newly accepted view and returns what starts now.
    pub(crate) fn on_view(&mut self, now_ms: u64, diff: ViewDiff<D>) -> Vec<Command<D>> {
        let finishing = diff.banner.is_some();
        let make = |kind, detail| Self::cue(kind, detail, now_ms, finishing);
        let mut cues = Vec::new();
        if finishing || diff.scene_changed {
            // The results of the last blow come before the scene changes behind them.
            cues.extend(diff.roll.map(|d| make(CueKind::Roll, Some(d))));
            cues.extend(diff.impact.map(|d| make(CueKind::Impact, Some(d))));
            cues.extend(diff.defeat.map(|d| make(CueKind::Defeat, Some(d))));
            cues.push(make(CueKind::Scene, None));
            cues.extend(diff.banner.map(|d| make(CueKind::Banner, Some(d))));
        } else {
            cues.push(Self::cue(CueKind::Render, None, now_ms, false));
            cues.extend(
                diff.heal
                    .map(|d| Self::cue(CueKind::Heal, Some(d), now_ms, false)),
            );
            cues.extend(
                diff.roll
                    .map(|d| Self::cue(CueKind::Roll, Some(d), now_ms, false)),
            );
            cues.extend(
                diff.impact
                    .map(|d| Self::cue(CueKind::Impact, Some(d), now_ms, false)),
            );
            cues.extend(
                diff.defeat
                    .map(|d| Self::cue(CueKind::Defeat, Some(d), now_ms, false)),
            );
        }
        for (kind, line) in [
            (CueKind::Narrate, diff.narration),
            (CueKind::Speak, diff.speech),
        ] {
            if let Some((detail, hold_ms)) = line {
                let mut cue = Self::cue(kind, Some(detail), now_ms, false);
                cue.hold_ms = hold_ms;
                cue.voiced = diff.voiced;
                cues.push(cue);
            }
        }
        self.queue.extend(cues);
        self.collapse_if_behind(now_ms);
        self.advance(now_ms)
    }

    pub(crate) fn on_tick(&mut self, now_ms: u64) -> Vec<Command<D>> {
        self.collapse_if_behind(now_ms);
        self.advance(now_ms)
    }

    /// A voice clip ended (or another awaited signal arrived) for the current line.
    pub(crate) fn on_cue_finished(&mut self, now_ms: u64, kind: CueKind) -> Vec<Command<D>> {
        if let Some(active) = self.active.as_mut()
            && active.kind == kind
            && active.waits_for_signal
        {
            active.signalled = true;
        }
        self.advance(now_ms)
    }

    /// Drops stale cosmetic cues when the queue is long or the display is far behind. The
    /// outcome-bearing cues stay, shortened; the newest view is still drawn by the last
    /// `Render`/`Scene`.
    fn collapse_if_behind(&mut self, now_ms: u64) {
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
            if cue.outcome {
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
            kept.push(Self::cue(CueKind::Render, None, now_ms, false));
        }
        self.queue = kept.into();
    }

    fn begin(&mut self, now_ms: u64, cue: &Cue<D>, state: StageState) -> (Active<D>, Command<D>) {
        let (minimum_ms, maximum_ms, waits) = match state {
            StageState::Rolling => (
                shorten(1_150, cue.short, 3),
                shorten(1_150, cue.short, 3),
                false,
            ),
            StageState::Impact => (
                shorten(650, cue.short, 3),
                shorten(650, cue.short, 3),
                false,
            ),
            StageState::Defeat => (
                shorten(1_100, cue.short, 3),
                shorten(1_100, cue.short, 3),
                false,
            ),
            StageState::Healing => (900, 900, false),
            StageState::LeavingScene => (
                shorten(350, cue.short, 2),
                shorten(350, cue.short, 2),
                false,
            ),
            StageState::Interstitial => (
                shorten(700, cue.short, 2),
                shorten(700, cue.short, 2),
                false,
            ),
            StageState::EnteringScene => (
                shorten(800, cue.short, 2),
                shorten(800, cue.short, 2),
                false,
            ),
            StageState::Aftermath => (
                shorten(1_200, cue.short, 3),
                shorten(1_200, cue.short, 3),
                false,
            ),
            StageState::Narrating | StageState::Speaking if cue.voiced => (
                VOICE_MINIMUM_MS.min(cue.hold_ms),
                cue.hold_ms + VOICE_GRACE_MS,
                true,
            ),
            StageState::Narrating | StageState::Speaking => (cue.hold_ms, cue.hold_ms, false),
            StageState::Idle => (0, 0, false),
        };
        let active = Active {
            kind: cue.kind,
            state,
            detail: cue.detail.clone(),
            started_ms: now_ms,
            minimum_ms,
            maximum_ms,
            waits_for_signal: waits,
            signalled: false,
            short: cue.short,
            voiced: cue.voiced,
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
        if elapsed >= u64::from(active.maximum_ms) {
            return true;
        }
        if active.waits_for_signal {
            return active.signalled && elapsed >= u64::from(active.minimum_ms);
        }
        elapsed >= u64::from(active.minimum_ms)
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
                let ended_at = if ended.waits_for_signal {
                    now_ms
                } else {
                    (ended.started_ms + u64::from(ended.maximum_ms)).min(now_ms)
                };
                if let Some(next) = self.next_phase(&ended) {
                    self.start_phase(ended_at, next, &mut commands);
                    continue;
                }
                ended_any = true;
            }
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
                CueKind::Narrate => StageState::Narrating,
                CueKind::Speak => StageState::Speaking,
                CueKind::Banner => StageState::Aftermath,
            };
            self.start_phase(now_ms, (cue, state), &mut commands);
        }
        commands
    }

    /// The state that follows a scene change's leaving and interstitial phases.
    fn next_phase(&self, ended: &Active<D>) -> Option<(Cue<D>, StageState)> {
        let next = match ended.state {
            StageState::LeavingScene => StageState::Interstitial,
            StageState::Interstitial => StageState::EnteringScene,
            _ => return None,
        };
        Some((
            Cue {
                kind: CueKind::Scene,
                detail: ended.detail.clone(),
                hold_ms: 0,
                voiced: ended.voiced,
                outcome: false,
                short: ended.short,
                queued_ms: ended.started_ms,
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
                narration: Some(("narration", 2_000)),
                scene_changed: true,
                ..ViewDiff::default()
            },
        ));
        commands.extend(run(&mut director, 60, 20_000));
        let order = states(&commands);
        // The sequence outlasts the lag limit, so the cosmetic narration may be collapsed,
        // but the outcome cues all play, in order.
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
    fn a_voiced_line_whose_end_never_arrives_times_out() {
        let mut director = Director::new();
        let mut commands = director.on_view(
            0,
            ViewDiff {
                speech: Some(("line", 2_000)),
                voiced: true,
                ..ViewDiff::default()
            },
        );
        commands.extend(run(&mut director, 50, 2_950));
        commands.extend(director.on_view(
            3_000,
            ViewDiff {
                heal: Some("heal"),
                ..ViewDiff::default()
            },
        ));
        assert_eq!(director.state(), StageState::Speaking);
        commands.extend(run(&mut director, 3_050, 4_900));
        assert_eq!(director.state(), StageState::Speaking);
        commands.extend(run(&mut director, 4_950, 8_000));
        assert!(states(&commands).iter().any(|name| name == "Healing"));
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn a_voiced_line_ends_early_when_its_clip_ends() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                narration: Some(("narration", 6_000)),
                voiced: true,
                ..ViewDiff::default()
            },
        );
        assert_eq!(director.state(), StageState::Narrating);
        director.on_tick(500);
        assert_eq!(director.state(), StageState::Narrating);
        director.on_cue_finished(1_500, CueKind::Narrate);
        assert_eq!(director.state(), StageState::Idle);
    }

    #[test]
    fn backlog_collapse_keeps_outcome_cues_in_shortened_form() {
        let mut director = Director::new();
        director.on_view(
            0,
            ViewDiff {
                speech: Some(("blocker", 5_000)),
                ..ViewDiff::default()
            },
        );
        for at in 1..=12 {
            director.on_view(
                at,
                ViewDiff {
                    roll: Some("cosmetic roll"),
                    heal: Some("cosmetic heal"),
                    narration: Some(("cosmetic line", 3_000)),
                    ..ViewDiff::default()
                },
            );
        }
        let mut commands = director.on_view(20, killing_blow());
        assert!(
            director.queue_len() <= QUEUE_CAP,
            "{}",
            director.queue_len()
        );
        commands.extend(run(&mut director, 70, 20_000));
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
                speech: Some(("blocker", 30_000)),
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
}
