//! The running stage: tokens, the camera rig, playback of director timelines, floating numbers
//! and effects, advanced by explicit elapsed time so every behaviour is testable with a
//! controlled clock. The renderer asks [`Stage::frame`] for a plain description of what to draw.
//! Nothing here decides an outcome; beats arrive already resolved by the server.

use std::{collections::VecDeque, time::Duration};

use super::{
    camera::{CameraPose, CameraRig, ShotName},
    director::{self, Anim, FlashTone, PlanError, Step, Timeline},
    grid::{Cell, Grid},
    math::{Vec3, ease_in_out},
    model::{
        Beat, CameraCommand, CameraFocus, FLOAT_LIFETIME_S, FinishingBlow, FloatingText, Highlight,
        HitPoints, HudState, Side, Token, TokenId,
    },
    scene_file::BattleScene,
};

/// Beats waiting behind the one playing. A further beat is refused, not dropped silently.
pub const MAX_QUEUED_BEATS: usize = 8;
const FALLEN_LEAN: f32 = 1.3;
const FALLEN_ALPHA: f32 = 0.42;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageError {
    UnknownToken(TokenId),
    QueueFull,
}

struct Walk {
    points: Vec<Vec3>,
    cells: Vec<Cell>,
    per_step_s: f32,
    elapsed_s: f32,
}

struct ActiveAnim {
    anim: Anim,
    elapsed_s: f32,
    /// Unit ground direction toward the opponent (lunge) or the attacker (recoil, dodge).
    direction: Vec3,
}

struct Runtime {
    token: Token,
    position: Vec3,
    walk: Option<Walk>,
    anim: Option<ActiveAnim>,
    downed: bool,
    shown_hp: Option<HitPoints>,
    /// While set, the server's newer hit points wait in `pending_hp` until the blow lands on
    /// screen (`ShowHitPoints`) or this many seconds pass.
    hp_hold_s: Option<f32>,
    pending_hp: Option<HitPoints>,
    phase: f32,
}

/// How long a hit-point change waits for its blow to be staged before the bar catches up anyway.
pub const HP_HOLD_SECONDS: f32 = 8.0;
/// How long the roll panel stays up.
pub const ROLL_CARD_SECONDS: f32 = 3.4;

struct Playback {
    timeline: Timeline,
    next: usize,
    elapsed: Duration,
}

#[derive(Clone, Copy, Debug)]
struct Flash {
    tone: FlashTone,
    strength: f32,
    age_s: f32,
}

/// One figure as the renderer draws it. `position` is the point on the ground between its feet.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureView {
    pub id: TokenId,
    pub side: Side,
    pub name: String,
    pub position: Vec3,
    pub height_m: f32,
    /// Rotation about the feet in the picture plane, radians; positive tips toward screen right.
    pub lean: f32,
    pub alpha: f32,
    pub red_flash: f32,
    pub heal_glow: f32,
    /// Vertical scale from breathing, around 1.
    pub breath: f32,
    pub tint: [f32; 3],
    pub active: bool,
    pub figure: Option<String>,
    pub portrait: Option<String>,
    pub hit_points: Option<HitPoints>,
    /// How far the figure has left its resting cell, used to widen its contact shadow.
    pub lift: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlashView {
    pub tone: FlashTone,
    pub strength: f32,
}

pub struct FrameView<'a> {
    pub camera: CameraPose,
    pub shake_px: (f32, f32),
    pub fade: f32,
    /// Far to near, so alpha blending is correct.
    pub figures: Vec<FigureView>,
    pub highlight: Option<&'a Highlight>,
    pub flash: Option<FlashView>,
    pub floats: &'a [FloatingText],
    pub hud: &'a HudState,
    pub clock_s: f32,
}

pub struct Stage {
    scene: BattleScene,
    tokens: Vec<Runtime>,
    rig: CameraRig,
    playing: Option<Playback>,
    queue: VecDeque<Beat>,
    pending_tokens: Option<Vec<Token>>,
    highlight: Option<Highlight>,
    hud: HudState,
    floats: Vec<FloatingText>,
    flash: Option<Flash>,
    clock_s: f32,
    reduced_motion: bool,
    dropped_beats: u32,
    next_float_id: u64,
    roll_card_left_s: f32,
}

fn flat_direction(from: Vec3, to: Vec3) -> Vec3 {
    let delta = Vec3::new(to.x - from.x, 0.0, to.z - from.z);
    let unit = delta.normalized();
    if unit == Vec3::ZERO {
        Vec3::new(0.0, 0.0, -1.0)
    } else {
        unit
    }
}

impl Stage {
    pub fn new(scene: BattleScene, reduced_motion: bool) -> Self {
        let opening = scene
            .cameras
            .get(&ShotName::Tactical)
            .copied()
            .expect("scene files are validated to hold every named shot");
        Self {
            rig: CameraRig::new(opening, reduced_motion),
            scene,
            tokens: Vec::new(),
            playing: None,
            queue: VecDeque::new(),
            pending_tokens: None,
            highlight: None,
            hud: HudState::default(),
            floats: Vec::new(),
            flash: None,
            clock_s: 0.0,
            reduced_motion,
            dropped_beats: 0,
            next_float_id: 0,
            roll_card_left_s: 0.0,
        }
    }

    pub fn scene(&self) -> &BattleScene {
        &self.scene
    }

    pub fn grid(&self) -> &Grid {
        &self.scene.grid
    }

    pub fn hud(&self) -> &HudState {
        &self.hud
    }

    pub fn set_round(&mut self, round: u32) {
        self.hud.round = round;
    }

    pub fn show_roll_card(&mut self, card: super::model::RollCard) {
        self.hud.roll_card = Some(card);
        self.roll_card_left_s = ROLL_CARD_SECONDS;
    }

    pub fn set_initiative(&mut self, order: Vec<TokenId>) {
        self.hud.initiative = order;
    }

    pub fn set_active(&mut self, active: Option<TokenId>) {
        self.hud.active = active;
    }

    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
        self.rig.set_reduced_motion(reduced);
    }

    /// Beats that could not be planned when their turn came (a token vanished meanwhile).
    pub fn dropped_beats(&self) -> u32 {
        self.dropped_beats
    }

    pub fn is_busy(&self) -> bool {
        self.playing.is_some() || !self.queue.is_empty()
    }

    pub fn tokens(&self) -> impl Iterator<Item = &Token> {
        self.tokens.iter().map(|runtime| &runtime.token)
    }

    pub fn camera_pose(&self) -> CameraPose {
        self.rig.pose()
    }

    /// Opens on the wide tactical shot and eases to the combat establishing shot.
    pub fn establish(&mut self) {
        if let Some(wide) = self.scene.cameras.get(&ShotName::Tactical) {
            self.rig.cut_to(*wide);
        }
        if let Some(combat) = self.scene.cameras.get(&ShotName::CombatEst) {
            self.rig.pan_to(*combat, Duration::from_millis(2600));
        }
    }

    /// Replaces the set of combatants. While a beat is playing, the new set is held and applied
    /// when the stage goes idle so a late view cannot teleport a figure mid-step.
    pub fn set_tokens(&mut self, tokens: Vec<Token>) {
        if self.is_busy() {
            self.pending_tokens = Some(tokens);
            return;
        }
        self.pending_tokens = None;
        self.tokens
            .retain(|runtime| tokens.iter().any(|token| token.id == runtime.token.id));
        for incoming in tokens {
            let centre = self.scene.grid.cell_centre(incoming.cell);
            match self
                .tokens
                .iter_mut()
                .find(|runtime| runtime.token.id == incoming.id)
            {
                Some(runtime) => {
                    if runtime.token.cell != incoming.cell {
                        runtime.position = centre;
                    }
                    if runtime.hp_hold_s.is_some() {
                        runtime.pending_hp = incoming.hit_points;
                    } else {
                        runtime.shown_hp = incoming.hit_points;
                        runtime.downed = incoming.hit_points.is_some_and(|hp| hp.current == 0);
                    }
                    runtime.token = incoming;
                }
                None => {
                    let downed = incoming.hit_points.is_some_and(|hp| hp.current == 0);
                    let phase = self.tokens.len() as f32 * 1.7;
                    self.tokens.push(Runtime {
                        position: centre,
                        walk: None,
                        anim: None,
                        downed,
                        shown_hp: incoming.hit_points,
                        hp_hold_s: None,
                        pending_hp: None,
                        phase,
                        token: incoming,
                    });
                }
            }
        }
    }

    /// Keeps the shown hit points of these tokens until their blow is staged, so a bar never
    /// drops before the hit it belongs to. A hold expires after [`HP_HOLD_SECONDS`].
    pub fn hold_hit_points<'a>(&mut self, ids: impl IntoIterator<Item = &'a TokenId>) {
        for id in ids {
            if let Some(index) = self.token_index(id) {
                self.tokens[index].hp_hold_s = Some(HP_HOLD_SECONDS);
            }
        }
    }

    /// Queues a resolved beat for staging.
    pub fn stage(&mut self, beat: Beat) -> Result<(), StageError> {
        let known = |id: &TokenId| self.tokens.iter().any(|runtime| &runtime.token.id == id);
        let missing = match &beat {
            Beat::TurnStart { actor } => (!known(actor)).then(|| actor.clone()),
            Beat::Attack {
                attacker, target, ..
            }
            | Beat::Approach { attacker, target }
            | Beat::Strike {
                attacker, target, ..
            } => [attacker, target]
                .into_iter()
                .find(|id| !known(id))
                .cloned(),
            Beat::Heal { source, target, .. } => {
                [source, target].into_iter().find(|id| !known(id)).cloned()
            }
        };
        if let Some(id) = missing {
            return Err(StageError::UnknownToken(id));
        }
        if self.queue.len() >= MAX_QUEUED_BEATS {
            return Err(StageError::QueueFull);
        }
        self.queue.push_back(beat);
        Ok(())
    }

    fn resting_tokens(&self) -> Vec<Token> {
        self.tokens
            .iter()
            .map(|runtime| runtime.token.clone())
            .collect()
    }

    fn start_next(&mut self) {
        while let Some(beat) = self.queue.pop_front() {
            match director::plan(&self.scene.grid, &self.resting_tokens(), &beat) {
                Ok(timeline) => {
                    self.playing = Some(Playback {
                        timeline,
                        next: 0,
                        elapsed: Duration::ZERO,
                    });
                    return;
                }
                Err(PlanError::UnknownToken(_)) => self.dropped_beats += 1,
            }
        }
    }

    /// Advances every clock by `elapsed`.
    pub fn update(&mut self, elapsed: Duration) {
        let dt = elapsed.as_secs_f32().min(0.25);
        self.clock_s += dt;
        self.rig.update(elapsed);
        if self.playing.is_none() {
            self.start_next();
        }
        self.advance_playback(elapsed);
        for runtime in &mut self.tokens {
            advance_runtime(runtime, dt);
            if let Some(left) = runtime.hp_hold_s {
                if left <= dt {
                    // The blow never reached the screen: let the bar catch up with the server.
                    runtime.hp_hold_s = None;
                    if let Some(pending) = runtime.pending_hp.take() {
                        runtime.shown_hp = Some(pending);
                        runtime.downed = pending.current == 0;
                    }
                } else {
                    runtime.hp_hold_s = Some(left - dt);
                }
            }
        }
        if self.roll_card_left_s > 0.0 {
            self.roll_card_left_s -= dt;
            if self.roll_card_left_s <= 0.0 {
                self.hud.roll_card = None;
            }
        }
        for text in &mut self.floats {
            text.age_s += dt;
        }
        self.floats.retain(|text| text.age_s < FLOAT_LIFETIME_S);
        if let Some(flash) = &mut self.flash {
            flash.age_s += dt;
            let life = if flash.tone == FlashTone::Heal {
                0.8
            } else {
                0.4
            };
            if flash.age_s >= life {
                self.flash = None;
            }
        }
        if !self.is_busy()
            && let Some(tokens) = self.pending_tokens.take()
        {
            self.set_tokens(tokens);
        }
    }

    fn advance_playback(&mut self, elapsed: Duration) {
        let Some(mut playback) = self.playing.take() else {
            return;
        };
        playback.elapsed += elapsed;
        while let Some(timed) = playback.timeline.steps.get(playback.next) {
            if timed.at > playback.elapsed {
                break;
            }
            let step = timed.step.clone();
            playback.next += 1;
            self.apply(step);
        }
        if playback.elapsed >= playback.timeline.duration
            && playback.next >= playback.timeline.steps.len()
        {
            self.playing = None;
            self.start_next();
        } else {
            self.playing = Some(playback);
        }
    }

    fn token_index(&self, id: &TokenId) -> Option<usize> {
        self.tokens
            .iter()
            .position(|runtime| &runtime.token.id == id)
    }

    fn focus_point(&self, focus: &CameraFocus) -> Option<Vec3> {
        let chest = |id: &TokenId| {
            self.token_index(id).map(|index| {
                let runtime = &self.tokens[index];
                runtime.position + Vec3::new(0.0, runtime.token.clamped_height() * 0.5, 0.0)
            })
        };
        match focus {
            CameraFocus::Scene => None,
            CameraFocus::Token(id) => chest(id),
            CameraFocus::Between(a, b) => Some(chest(a)?.lerp(chest(b)?, 0.5)),
        }
    }

    fn camera_pose_for(&self, command: &CameraCommand) -> Option<CameraPose> {
        let base = *self.scene.cameras.get(&command.shot)?;
        let aimed = match self.focus_point(&command.focus) {
            Some(point) => base.aimed_at(point),
            None => base,
        };
        Some(aimed.dollied(command.dolly))
    }

    fn apply(&mut self, step: Step) {
        match step {
            Step::SetActive(active) => self.hud.active = active,
            Step::Camera { command, pan } => {
                if let Some(pose) = self.camera_pose_for(&command) {
                    self.rig.pan_to(pose, pan);
                }
            }
            Step::Highlight(highlight) => self.highlight = highlight,
            Step::MoveTo {
                token,
                cell,
                per_step,
            } => self.move_to(&token, cell, per_step),
            Step::Animate { token, anim } => self.start_anim(&token, anim),
            Step::Shake {
                amplitude_px,
                duration,
            } => self.rig.shake(amplitude_px, duration),
            Step::Flash { tone, strength } => {
                self.flash = Some(Flash {
                    tone,
                    // Reduced motion softens the flash into a gentle fade.
                    strength: if self.reduced_motion {
                        strength * 0.35
                    } else {
                        strength
                    },
                    age_s: 0.0,
                });
            }
            Step::Float { token, text, tone } => {
                self.next_float_id += 1;
                self.floats.push(FloatingText {
                    id: self.next_float_id,
                    token,
                    text,
                    tone,
                    age_s: 0.0,
                })
            }
            Step::ShowHitPoints { token, current } => {
                if let Some(index) = self.token_index(&token) {
                    let runtime = &mut self.tokens[index];
                    let maximum = runtime.shown_hp.map_or(current.max(1), |hp| hp.maximum);
                    runtime.shown_hp = Some(HitPoints {
                        current: current.min(maximum),
                        maximum,
                    });
                    runtime.hp_hold_s = None;
                    runtime.pending_hp = None;
                }
            }
            Step::FinishingBlow { hero, enemy } => {
                let name = |id: &TokenId| {
                    self.token_index(id)
                        .map(|index| self.tokens[index].token.name.clone())
                        .unwrap_or_default()
                };
                self.hud.finishing_blow = Some(FinishingBlow {
                    hero: name(&hero),
                    enemy: name(&enemy),
                });
            }
            Step::EndFinishingBlow => self.hud.finishing_blow = None,
        }
    }

    /// Walks the token from where it stands to `cell`. Its formation cell does not change: the
    /// token's `cell` stays home so the strike can send it back.
    fn move_to(&mut self, id: &TokenId, cell: Cell, per_step: Duration) {
        let Some(index) = self.token_index(id) else {
            return;
        };
        let reduced = self.reduced_motion;
        let grid = &self.scene.grid;
        let target = grid.cell_centre(cell);
        let runtime = &mut self.tokens[index];
        let from = grid.cell_at(runtime.position).unwrap_or(runtime.token.cell);
        let path = grid.path(from, cell).unwrap_or_default();
        if reduced || per_step.is_zero() {
            runtime.walk = None;
            runtime.position = target;
            return;
        }
        if path.len() < 2 {
            if (runtime.position - target).length() > 0.02 {
                runtime.position = target;
            }
            return;
        }
        let steps: Vec<Cell> = path[1..].to_vec();
        let mut points = vec![runtime.position];
        points.extend(steps.iter().map(|step| grid.cell_centre(*step)));
        runtime.walk = Some(Walk {
            points,
            cells: steps,
            per_step_s: per_step.as_secs_f32().max(0.01),
            elapsed_s: 0.0,
        });
    }

    fn start_anim(&mut self, id: &TokenId, anim: Anim) {
        let Some(index) = self.token_index(id) else {
            return;
        };
        if matches!(anim, Anim::Fall) && self.reduced_motion {
            self.tokens[index].downed = true;
            return;
        }
        let here = self.tokens[index].position;
        let other = match &anim {
            Anim::Lunge { toward: other }
            | Anim::Recoil { from: other }
            | Anim::Dodge { from: other } => self
                .token_index(other)
                .map(|other| self.tokens[other].position),
            Anim::Fall | Anim::Heal => None,
        };
        let direction = match (&anim, other) {
            (Anim::Lunge { .. }, Some(other)) => flat_direction(here, other),
            (Anim::Recoil { .. } | Anim::Dodge { .. }, Some(other)) => flat_direction(other, here),
            _ => Vec3::new(0.0, 0.0, -1.0),
        };
        self.tokens[index].anim = Some(ActiveAnim {
            anim,
            elapsed_s: 0.0,
            direction,
        });
    }

    /// Projects a world point to normalised screen coordinates (0..1, y down), or `None` behind
    /// the camera. Used to anchor floating numbers and name tags in the HUD.
    pub fn project(&self, point: Vec3, aspect: f32) -> Option<(f32, f32)> {
        let pose = self.rig.pose();
        let matrix = pose.projection(aspect, 0.1, 200.0).mul(&pose.view());
        let (x, y, _) = matrix.project(point)?;
        Some((x * 0.5 + 0.5, 0.5 - y * 0.5))
    }

    pub fn frame(&self) -> FrameView<'_> {
        let eye = self.rig.pose().eye();
        let mut figures: Vec<FigureView> = self
            .tokens
            .iter()
            .map(|runtime| self.figure(runtime))
            .collect();
        figures.sort_by(|a, b| {
            let distance = |figure: &FigureView| (figure.position - eye).length();
            distance(b).total_cmp(&distance(a))
        });
        FrameView {
            camera: self.rig.pose(),
            shake_px: self.rig.shake_offset_px(),
            fade: self.rig.transition_fade(),
            figures,
            highlight: self.highlight.as_ref(),
            flash: self.flash.map(|flash| FlashView {
                tone: flash.tone,
                strength: flash.strength
                    * (1.0
                        - flash.age_s
                            / if flash.tone == FlashTone::Heal {
                                0.8
                            } else {
                                0.4
                            })
                    .clamp(0.0, 1.0),
            }),
            floats: &self.floats,
            hud: &self.hud,
            clock_s: self.clock_s,
        }
    }

    fn figure(&self, runtime: &Runtime) -> FigureView {
        let mut position = runtime.position;
        let mut lean = 0.0;
        let mut alpha = 1.0;
        let mut red_flash = 0.0;
        let mut heal_glow = 0.0;
        let reach = self.scene.grid.cell_m() * 0.5;
        if let Some(active) = &runtime.anim {
            let duration = active.anim.duration().as_secs_f32();
            let progress = (active.elapsed_s / duration).clamp(0.0, 1.0);
            let arc = (std::f32::consts::PI * progress).sin();
            match &active.anim {
                Anim::Lunge { .. } => position = position + active.direction * (reach * arc),
                Anim::Recoil { .. } => {
                    position = position + active.direction * (0.32 * arc);
                    lean = 0.14 * arc * active.direction.x.signum();
                    red_flash = 1.0 - progress;
                }
                Anim::Dodge { .. } => {
                    let side = Vec3::new(-active.direction.z, 0.0, active.direction.x);
                    position = position + side * (0.5 * arc);
                }
                Anim::Fall => {
                    let eased = ease_in_out(progress);
                    lean = FALLEN_LEAN * eased;
                    alpha = 1.0 - (1.0 - FALLEN_ALPHA) * eased;
                    red_flash = (1.0 - progress) * 0.6;
                }
                Anim::Heal => {
                    heal_glow = arc;
                    position.y += 0.05 * arc;
                }
            }
        }
        if runtime.downed && !matches!(runtime.anim.as_ref().map(|a| &a.anim), Some(Anim::Fall)) {
            lean = FALLEN_LEAN;
            alpha = FALLEN_ALPHA;
        }
        if let Some(walk) = &runtime.walk {
            position = walk_position(walk);
        }
        let mut tint = [1.0, 1.0, 1.0];
        for status in &runtime.token.statuses {
            let colour = status.tint();
            for (channel, value) in tint.iter_mut().zip(colour) {
                *channel *= value;
            }
        }
        let rest = self.scene.grid.cell_centre(runtime.token.cell);
        FigureView {
            id: runtime.token.id.clone(),
            side: runtime.token.side,
            name: runtime.token.name.clone(),
            position,
            height_m: runtime.token.clamped_height(),
            lean,
            alpha,
            red_flash,
            heal_glow,
            breath: if runtime.downed {
                1.0
            } else {
                1.0 + 0.012 * (self.clock_s * 2.2 + runtime.phase).sin()
            },
            tint,
            active: self.hud.active.as_ref() == Some(&runtime.token.id),
            figure: runtime.token.figure.clone(),
            portrait: runtime.token.portrait.clone(),
            hit_points: runtime.shown_hp,
            lift: ((position - rest).length() / reach.max(0.01)).min(2.0),
        }
    }
}

fn walk_position(walk: &Walk) -> Vec3 {
    let segment = walk.elapsed_s / walk.per_step_s;
    let index = (segment.floor() as usize).min(walk.points.len() - 2);
    let fraction = (segment - index as f32).clamp(0.0, 1.0);
    let mut point = walk.points[index].lerp(walk.points[index + 1], fraction);
    point.y += 0.07 * (std::f32::consts::PI * fraction).sin();
    point
}

fn advance_runtime(runtime: &mut Runtime, dt: f32) {
    if let Some(walk) = &mut runtime.walk {
        walk.elapsed_s += dt;
        if walk.elapsed_s >= walk.per_step_s * walk.cells.len() as f32 {
            if let Some(end) = walk.points.last() {
                runtime.position = *end;
            }
            runtime.walk = None;
        }
    }
    if let Some(active) = &mut runtime.anim {
        active.elapsed_s += dt;
        if active.elapsed_s >= active.anim.duration().as_secs_f32() {
            if matches!(active.anim, Anim::Fall) {
                runtime.downed = true;
            }
            runtime.anim = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::{
        model::{AttackResult, FloatTone, HitPoints},
        scene_file::load_embedded,
    };

    fn token(id: &str, side: Side, cell: Cell, hp: u32) -> Token {
        Token {
            id: TokenId::new(id),
            side,
            name: id.to_uppercase(),
            cell,
            height_m: 1.8,
            figure: None,
            portrait: None,
            hit_points: Some(HitPoints {
                current: hp,
                maximum: 12,
            }),
            statuses: Vec::new(),
        }
    }

    fn stage(reduced: bool) -> Stage {
        let scene = load_embedded("salt-quay").unwrap().unwrap();
        let mut stage = Stage::new(scene, reduced);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        stage
    }

    fn run(stage: &mut Stage, seconds: f32) {
        let ticks = (seconds / 0.05) as u32;
        for _ in 0..ticks {
            stage.update(Duration::from_millis(50));
        }
    }

    fn figure(stage: &Stage, id: &str) -> FigureView {
        stage
            .frame()
            .figures
            .into_iter()
            .find(|figure| figure.id.0 == id)
            .unwrap()
    }

    fn killing_blow() -> Beat {
        Beat::Attack {
            attacker: TokenId::new("hero"),
            target: TokenId::new("enemy"),
            result: AttackResult::Hit {
                damage: 9,
                critical: false,
                downed: true,
            },
            target_hit_points_after: Some(0),
        }
    }

    #[test]
    fn figures_start_standing_on_their_cell_centres() {
        let stage = stage(false);
        let hero = figure(&stage, "hero");
        assert_eq!(hero.position, stage.grid().cell_centre(Cell::new(3, 4)));
        assert_eq!((hero.lean, hero.alpha), (0.0, 1.0));
    }

    #[test]
    fn frame_lists_figures_far_to_near() {
        let stage = stage(false);
        let order: Vec<String> = stage
            .frame()
            .figures
            .iter()
            .map(|f| f.id.0.clone())
            .collect();
        assert_eq!(order[0], "enemy", "the far enemy is drawn first");
    }

    #[test]
    fn a_downing_blow_plays_through_walk_impact_fall_and_the_finishing_card() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        let mut saw_walk_away_from_cell = false;
        let mut saw_float = false;
        let mut saw_card_at = None;
        let mut saw_card_end = false;
        let mut shake_seen = false;
        for tick in 0..400 {
            stage.update(Duration::from_millis(50));
            let frame = stage.frame();
            saw_walk_away_from_cell |= frame.figures.iter().any(|f| {
                f.id.0 == "hero"
                    && (f.position - stage.grid().cell_centre(Cell::new(3, 4))).length() > 0.5
            });
            saw_float |= frame.floats.iter().any(|text| text.text == "9");
            shake_seen |= frame.shake_px != (0.0, 0.0);
            if frame.hud.finishing_blow.is_some() && saw_card_at.is_none() {
                saw_card_at = Some(tick);
            }
            if saw_card_at.is_some() && frame.hud.finishing_blow.is_none() {
                saw_card_end = true;
            }
        }
        assert!(saw_walk_away_from_cell && saw_float && shake_seen);
        assert!(saw_card_at.is_some() && saw_card_end);
        let enemy = figure(&stage, "enemy");
        assert!(
            enemy.lean > 1.2 && enemy.alpha < 0.5,
            "enemy lies faded: {enemy:?}"
        );
        assert_eq!(enemy.hit_points.unwrap().current, 0);
        assert!(!stage.is_busy());
    }

    #[test]
    fn held_hit_points_wait_for_the_impact_then_expire_if_the_blow_never_comes() {
        let mut stage = stage(false);
        let enemy = TokenId::new("enemy");
        stage.hold_hit_points([&enemy]);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 2),
        ]);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 5);
        run(&mut stage, HP_HOLD_SECONDS + 1.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 2);
    }

    #[test]
    fn an_approach_then_a_strike_land_the_blow_after_the_step_and_return_home() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Approach {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
            })
            .unwrap();
        run(&mut stage, 1.3);
        let beside = figure(&stage, "hero").position;
        assert!(
            (beside - stage.grid().cell_centre(Cell::new(3, 4))).length() > 1.0,
            "stepped forward"
        );
        stage
            .stage(Beat::Strike {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Hit {
                    damage: 2,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(3),
            })
            .unwrap();
        run(&mut stage, 6.0);
        let hero = figure(&stage, "hero");
        assert!((hero.position - stage.grid().cell_centre(Cell::new(3, 4))).length() < 0.01);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 3);
    }

    #[test]
    fn the_hit_point_bar_changes_only_at_the_moment_of_impact() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 1.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 5);
        run(&mut stage, 3.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 0);
    }

    #[test]
    fn an_ordinary_hit_returns_the_attacker_to_formation() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Hit {
                    damage: 2,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(3),
            })
            .unwrap();
        run(&mut stage, 8.0);
        let hero = figure(&stage, "hero");
        assert!((hero.position - stage.grid().cell_centre(Cell::new(3, 4))).length() < 0.01);
        assert_eq!(hero.lean, 0.0);
        let enemy = figure(&stage, "enemy");
        assert_eq!(enemy.alpha, 1.0);
        assert_eq!(enemy.hit_points.unwrap().current, 3);
    }

    #[test]
    fn a_late_token_update_waits_until_the_beat_finishes() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 0.5);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(0, 0), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 0),
        ]);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 5);
        run(&mut stage, 10.0);
        assert_eq!(
            figure(&stage, "hero").position,
            stage.grid().cell_centre(Cell::new(0, 0))
        );
    }

    #[test]
    fn staging_a_beat_for_an_unknown_token_is_refused_and_the_queue_is_bounded() {
        let mut stage = stage(false);
        assert_eq!(
            stage.stage(Beat::TurnStart {
                actor: TokenId::new("ghost")
            }),
            Err(StageError::UnknownToken(TokenId::new("ghost")))
        );
        for _ in 0..MAX_QUEUED_BEATS {
            stage
                .stage(Beat::TurnStart {
                    actor: TokenId::new("hero"),
                })
                .unwrap();
        }
        assert_eq!(
            stage.stage(Beat::TurnStart {
                actor: TokenId::new("hero")
            }),
            Err(StageError::QueueFull)
        );
    }

    #[test]
    fn reduced_motion_cuts_the_camera_teleports_steps_and_drops_shake() {
        let mut stage = stage(true);
        stage.stage(killing_blow()).unwrap();
        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
            let frame = stage.frame();
            assert_eq!(frame.shake_px, (0.0, 0.0));
            assert!(frame.floats.iter().all(|text| text.tone != FloatTone::Miss));
        }
        let enemy = figure(&stage, "enemy");
        assert!(enemy.lean > 1.2);
        assert!(!stage.is_busy());
    }

    #[test]
    fn the_camera_ends_on_the_victory_shot_after_the_finishing_blow() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 20.0);
        let victory = stage.scene().cameras[&ShotName::Victory];
        let pose = stage.camera_pose();
        assert!((pose.yaw_deg - victory.yaw_deg).abs() < 0.01);
        assert!(pose.distance < victory.distance, "pushed in");
    }

    #[test]
    fn projecting_a_cell_centre_lands_inside_the_screen_for_the_establishing_shot() {
        let mut stage = stage(false);
        stage.establish();
        run(&mut stage, 3.0);
        for cell in [Cell::new(3, 4), Cell::new(4, 1)] {
            let (x, y) = stage
                .project(stage.grid().cell_centre(cell), 16.0 / 9.0)
                .unwrap();
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "{cell:?} at {x},{y}"
            );
        }
    }
}
