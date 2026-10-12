//! The running stage: tokens, the camera rig, playback of director timelines, floating numbers
//! and effects, advanced by explicit elapsed time so every behaviour is testable with a
//! controlled clock. The renderer asks [`Stage::frame`] for a plain description of what to draw.
//! Nothing here decides an outcome; beats arrive already resolved by the server.

use std::{collections::VecDeque, time::Duration};

use super::{
    camera::{CameraPose, CameraRig, ShotName},
    director::{self, Anim, FALL_LANDS, FlashTone, PlanError, ReadyStyle, Step, Timeline},
    grid::{Cell, Grid},
    health::{self, Reaction, Tone},
    layout::{Cover, Layout},
    math::{Vec3, ease_in_out},
    model::{
        AttackResult, BadgeView, Beat, CameraCommand, CameraFocus, DieView, FLOAT_LIFETIME_S,
        FinishingBlow, FloatingText, Highlight, HitPoints, HudState, InitiativeShow, Side, Token,
        TokenId,
    },
    scene_file::BattleScene,
    sound::{SoundEvent, SoundKind, voice_for},
    status::{self, Status, StatusPose, StatusSet},
};

/// Beats waiting behind the one playing. A further beat is refused, not dropped silently.
pub const MAX_QUEUED_BEATS: usize = 8;
/// Sound events waiting for the page to play them. Older ones are dropped, and counted, if the
/// page stops draining (a hidden tab).
pub const MAX_PENDING_SOUNDS: usize = 64;
/// How long the landing squash of a fallen figure takes to settle.
const THUD_SECONDS: f32 = 0.32;
/// Share of the fall at which the body reaches the ground (matches `FALL_LANDS`).
const FALL_KNEEL: f32 = 0.55;
/// How much a figure laid low is dimmed and greyed: with its health variant (already ashen and
/// slumped) hardly at all, in its authored art a little.
const LAID_LOW_DIM_WITH_VARIANT: f32 = 0.1;
const LAID_LOW_DIM: f32 = 0.32;
/// An arrow that missed stays stuck in the ground this long before it is gone.
const ARROW_STICK_SECONDS: f32 = 0.9;

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
    /// Footfalls already reported.
    landed: usize,
}

struct ActiveAnim {
    anim: Anim,
    elapsed_s: f32,
    /// Unit ground direction toward the opponent (lunge) or the attacker (recoil, dodge).
    direction: Vec3,
    /// A fall has reached the ground and squashed.
    landed: bool,
}

/// An arrow in the air or stuck in the ground.
struct Projectile {
    from: Vec3,
    to: Vec3,
    hits: bool,
    flight_s: f32,
    age_s: f32,
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
    /// The server-made health variant of this figure's picture that goes with `shown_hp`, so a
    /// picture never changes before the blow that causes it lands on screen.
    shown_variant: Option<String>,
    /// A bodily reaction in progress (a hit's shake, a slump) and how long it has played.
    reaction: Option<(Reaction, f32)>,
    /// The next blow on this figure is a critical hit (set when its beat is queued).
    critical_next: bool,
    /// Statuses the server reports, applied together with the hit points they came with so a
    /// condition never shows before the blow that causes it.
    server_status: StatusSet,
    pending_status: Option<StatusSet>,
    /// Seconds left of the landing squash after a fall.
    thud_s: f32,
    /// An archer aimed to her left last: her art (drawn aiming right) is mirrored.
    facing_left: bool,
    /// Stable per figure, so idle sway differs between figures and survives a roster refresh.
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

/// How long a tumbling initiative die spins before it shows its number.
const DIE_TUMBLE_SECONDS: f32 = 0.8;

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
    pub active: bool,
    /// Aimed at during the turn-focus shot: draw the ring and the name tag.
    pub targeted: bool,
    /// How far the figure has slumped to the ground (0 standing, 1 down), and the landing squash.
    pub slump: f32,
    pub thud: f32,
    /// On the ground for good (knocked out): chips and plates show it.
    pub downed: bool,
    pub dim: f32,
    pub statuses: StatusSet,
    pub pose: StatusPose,
    /// Cover next to the figure: direction on the ground toward it and how good it is.
    pub cover: Option<CoverPip>,
    /// A faint trailing copy offset sideways (the dodging afterimage), 0 when absent.
    pub afterimage: f32,
    /// Heartbeat pulse of a dying figure, 0..1.
    pub heartbeat: f32,
    pub phase: f32,
    /// Draw the cut-out mirrored (an archer aiming to her left).
    pub mirror: bool,
    pub figure: Option<String>,
    pub portrait: Option<String>,
    pub hit_points: Option<HitPoints>,
    /// The server-made health variant to draw instead of the authored art, once it is loaded.
    pub variant: Option<String>,
    /// How hurt the figure looks (from the installed classifier), for the plate and the tremor.
    pub tone: Option<Tone>,
    /// How far the figure has left its resting cell, used to widen its contact shadow.
    pub lift: f32,
}

/// The small shield shown on the tile edge facing cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverPip {
    pub toward: [f32; 2],
    pub level: Cover,
}

/// An arrow as the renderer draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileView {
    pub head: Vec3,
    pub tail: Vec3,
    pub trail: Vec3,
    pub alpha: f32,
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
    pub projectiles: Vec<ProjectileView>,
    /// The layout in play and the grid with its obstacles applied.
    pub layout: Option<&'a Layout>,
    pub grid: &'a Grid,
    pub flash: Option<FlashView>,
    pub floats: &'a [FloatingText],
    pub hud: &'a HudState,
    pub clock_s: f32,
    pub reduced_motion: bool,
    /// The restrained content rating: no blood in the effects.
    pub restrained: bool,
}

pub struct Stage {
    scene: BattleScene,
    /// The scene's grid with the chosen layout's obstacles applied.
    grid: Grid,
    layout: Option<usize>,
    sounds: VecDeque<SoundEvent>,
    dropped_sounds: u32,
    projectiles: Vec<Projectile>,
    target: Option<TokenId>,
    /// Whether the timeline now playing is the start-of-fight initiative sequence.
    initiative_playing: bool,
    restrained: bool,
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
            grid: scene.grid.clone(),
            layout: None,
            sounds: VecDeque::new(),
            dropped_sounds: 0,
            projectiles: Vec::new(),
            target: None,
            initiative_playing: false,
            restrained: false,
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

    /// The grid figures move on: the scene's grid with the layout's obstacles applied.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// The layout in play, if one was chosen.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.and_then(|index| self.scene.layouts.get(index))
    }

    /// Chooses the layout by id and rebuilds the movement grid. Returns `false` (leaving the
    /// current layout) when the scene has no such layout. Call it before placing tokens.
    pub fn set_layout(&mut self, id: &str) -> bool {
        let Some(index) = self.scene.layouts.iter().position(|layout| layout.id == id) else {
            return false;
        };
        self.layout = Some(index);
        self.grid = self.scene.grid.with_layout(&self.scene.layouts[index]);
        true
    }

    /// Takes the sounds that happened since the last call, in order.
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> {
        self.sounds.drain(..).collect()
    }

    /// Sounds discarded because nobody drained them in time.
    pub fn dropped_sounds(&self) -> u32 {
        self.dropped_sounds
    }

    fn emit(&mut self, kind: SoundKind, token: &TokenId) {
        self.emit_at_rate(kind, token, 1.0);
    }

    fn emit_at_rate(&mut self, kind: SoundKind, token: &TokenId, rate: f32) {
        let Some(index) = self.token_index(token) else {
            return;
        };
        let runtime = &self.tokens[index];
        let event = SoundEvent {
            kind,
            voice: voice_for(runtime.token.side, runtime.token.figure.as_deref()),
            surface: self.scene.surface,
            token: token.clone(),
            rate,
        };
        if self.sounds.len() >= MAX_PENDING_SOUNDS {
            self.sounds.pop_front();
            self.dropped_sounds += 1;
        }
        self.sounds.push_back(event);
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

    /// Whether the start-of-fight initiative sequence is playing.
    pub fn is_rolling_initiative(&self) -> bool {
        self.initiative_playing
    }

    /// Leaves the initiative sequence at once and cuts to the current turn: a newer accepted view
    /// shows someone already acted, so the sequence is history.
    pub fn skip_initiative(&mut self) {
        if !self.initiative_playing {
            return;
        }
        self.initiative_playing = false;
        self.playing = None;
        self.hud.roll_for_initiative = InitiativeShow::default();
        for runtime in &mut self.tokens {
            if matches!(
                runtime.anim.as_ref().map(|a| &a.anim),
                Some(Anim::Ready { .. })
            ) {
                runtime.anim = None;
            }
        }
        let focus = self.hud.active.clone().map(CameraFocus::Token);
        let command = CameraCommand {
            shot: ShotName::TurnFocus,
            focus: focus.unwrap_or(CameraFocus::Scene),
            dolly: 1.0,
        };
        if let Some(pose) = self.camera_pose_for(&command) {
            self.rig.cut_to(pose);
        }
    }

    /// Mutes blood in the figure and tile effects (the restrained content rating).
    pub fn set_restrained(&mut self, restrained: bool) {
        self.restrained = restrained;
    }

    pub fn restrained(&self) -> bool {
        self.restrained
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

    /// Holds the wide tactical shot (the layout stills frame the whole field with it).
    pub fn show_tactical(&mut self) {
        if let Some(wide) = self.scene.cameras.get(&ShotName::Tactical) {
            self.rig.cut_to(*wide);
        }
    }

    /// Holds a closer view of the field (the status contact sheets read figures with it).
    pub fn show_close(&mut self) {
        if let Some(pose) = self.scene.cameras.get(&ShotName::CombatEst) {
            self.rig.cut_to(pose.dollied(0.82));
        }
    }

    /// Moves a token to another cell at once (a review aid: server cells do the same in play).
    pub fn place_token(&mut self, id: &TokenId, cell: Cell) {
        if let Some(index) = self.token_index(id) {
            self.tokens[index].token.cell = cell;
            self.tokens[index].position = self.grid.cell_centre(cell);
            self.tokens[index].walk = None;
        }
    }

    /// Opens on the wide tactical shot and eases to the combat establishing shot.
    /// Cuts straight to `pose` (used by the preview to look at a scene from any angle).
    pub fn set_camera_pose(&mut self, pose: CameraPose) {
        self.rig.cut_to(pose);
    }

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
            let centre = self.grid.cell_centre(incoming.cell);
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
                        runtime.pending_status = Some(StatusSet::of(&incoming.statuses));
                    } else {
                        runtime.show_hp(incoming.hit_points);
                        runtime.downed = incoming.hit_points.is_some_and(|hp| hp.current == 0);
                        runtime.server_status = StatusSet::of(&incoming.statuses);
                    }
                    runtime.token = incoming;
                }
                None => {
                    let downed = incoming.hit_points.is_some_and(|hp| hp.current == 0);
                    let phase = phase_of(&incoming.id);
                    self.tokens.push(Runtime {
                        position: centre,
                        walk: None,
                        anim: None,
                        downed,
                        shown_hp: incoming.hit_points,
                        hp_hold_s: None,
                        pending_hp: None,
                        shown_variant: health::variant_for(&incoming.name),
                        reaction: None,
                        critical_next: false,
                        server_status: StatusSet::of(&incoming.statuses),
                        pending_status: None,
                        thud_s: 0.0,
                        facing_left: false,
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
            Beat::Initiative { rolls } => rolls
                .iter()
                .map(|roll| &roll.token)
                .find(|id| !known(id))
                .cloned(),
        };
        if let Some(id) = missing {
            return Err(StageError::UnknownToken(id));
        }
        if self.queue.len() >= MAX_QUEUED_BEATS {
            return Err(StageError::QueueFull);
        }
        if let Beat::Attack { target, result, .. } | Beat::Strike { target, result, .. } = &beat
            && let Some(index) = self.token_index(target)
        {
            self.tokens[index].critical_next =
                matches!(result, AttackResult::Hit { critical: true, .. });
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
            match director::plan_with(
                &self.grid,
                &self.resting_tokens(),
                &beat,
                self.reduced_motion,
            ) {
                Ok(timeline) => {
                    self.initiative_playing = matches!(beat, Beat::Initiative { .. });
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
        let mut footfalls = Vec::new();
        for runtime in &mut self.tokens {
            if runtime.hp_hold_s.is_none() {
                // The variant follows the shown hit points; while a blow is awaited it waits too.
                let published = health::variant_for(&runtime.token.name);
                if runtime.shown_variant != published {
                    runtime.shown_variant = published;
                }
            }
            if let Some((reaction, age)) = &mut runtime.reaction {
                *age += dt;
                if reaction.finished(*age) {
                    runtime.reaction = None;
                }
            }
            for _ in 0..advance_runtime(runtime, dt) {
                footfalls.push(runtime.token.id.clone());
            }
            if let Some(left) = runtime.hp_hold_s {
                if left <= dt {
                    // The blow never reached the screen: let the bar catch up with the server.
                    runtime.hp_hold_s = None;
                    if let Some(pending) = runtime.pending_hp.take() {
                        runtime.show_hp(Some(pending));
                        runtime.downed = pending.current == 0;
                    }
                    if let Some(status) = runtime.pending_status.take() {
                        runtime.server_status = status;
                    }
                } else {
                    runtime.hp_hold_s = Some(left - dt);
                }
            }
        }
        for id in footfalls {
            self.emit(SoundKind::Footstep, &id);
        }
        for runtime in &mut self.tokens {
            runtime.thud_s = (runtime.thud_s - dt).max(0.0);
            if let Some(active) = &mut runtime.anim
                && matches!(active.anim, Anim::Fall)
                && !active.landed
                && active.elapsed_s >= FALL_LANDS.as_secs_f32()
            {
                active.landed = true;
                runtime.thud_s = THUD_SECONDS;
            }
        }
        for die in &mut self.hud.roll_for_initiative.dice {
            die.age_s += dt;
            if die.age_s >= die.tumble_s {
                die.landed = true;
            }
        }
        for badge in &mut self.hud.roll_for_initiative.badges {
            badge.age_s += dt;
        }
        self.update_facing();
        for projectile in &mut self.projectiles {
            projectile.age_s += dt;
        }
        self.projectiles.retain(|projectile| {
            let life = projectile.flight_s
                + if projectile.hits {
                    0.0
                } else {
                    ARROW_STICK_SECONDS
                };
            projectile.age_s < life
        });
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

    /// An archer's art is drawn aiming to the right of the picture. She is mirrored when her
    /// target is on the left of the screen as the camera sees it now, with a little hysteresis so
    /// she does not flicker while the target is straight ahead.
    fn update_facing(&mut self) {
        let pose = self.rig.pose();
        let look = pose.target - pose.eye();
        let right = Vec3::new(-look.z, 0.0, look.x).normalized();
        for index in 0..self.tokens.len() {
            if self.tokens[index].token.attack != crate::battle::model::AttackStyle::Ranged {
                continue;
            }
            let aim = match &self.tokens[index].anim {
                Some(active) if matches!(active.anim, Anim::Draw { .. }) => Some(active.direction),
                _ => self
                    .nearest_opponent(&self.tokens[index])
                    .map(|point| flat_direction(self.tokens[index].position, point)),
            };
            if let Some(aim) = aim {
                let side = aim.x * right.x + aim.z * right.z;
                if side.abs() > 0.2 {
                    self.tokens[index].facing_left = side < 0.0;
                }
            }
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
            self.initiative_playing = false;
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
                    if let Some(status) = runtime.pending_status.take() {
                        runtime.server_status = status;
                    }
                    let maximum = runtime.shown_hp.map_or(current.max(1), |hp| hp.maximum);
                    runtime.show_hp(Some(HitPoints {
                        current: current.min(maximum),
                        maximum,
                    }));
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
            Step::Sound { kind, token } => self.emit(kind, &token),
            Step::Whip { command, travel } => {
                if let Some(pose) = self.camera_pose_for(&command) {
                    self.rig.whip_to(pose, travel);
                }
            }
            Step::Banner(text) => self.hud.roll_for_initiative.banner = text,
            Step::InitiativeStart => {
                self.hud.roll_for_initiative = InitiativeShow {
                    tracker: Some(Vec::new()),
                    ..InitiativeShow::default()
                };
            }
            Step::InitiativeDie {
                token,
                die,
                modifier,
                total,
            } => {
                let tumble_s = if self.reduced_motion {
                    0.0
                } else {
                    DIE_TUMBLE_SECONDS
                };
                self.hud.roll_for_initiative.dice.push(DieView {
                    token,
                    face: die,
                    modifier,
                    total,
                    age_s: 0.0,
                    tumble_s,
                    landed: tumble_s == 0.0,
                });
            }
            Step::InitiativeLand { token } => {
                for die in &mut self.hud.roll_for_initiative.dice {
                    if die.token == token {
                        die.landed = true;
                    }
                }
            }
            Step::ClearDice => self.hud.roll_for_initiative.dice.clear(),
            Step::InitiativeBadge {
                token,
                order,
                total,
            } => {
                // Only the current stop's badge stands; the tracker keeps the record. The badges
                // of one sweep arrive together and stay together.
                self.hud
                    .roll_for_initiative
                    .badges
                    .retain(|badge| badge.age_s < 0.2);
                let name = self
                    .token_index(&token)
                    .map(|index| self.tokens[index].token.name.clone())
                    .unwrap_or_default();
                self.hud.roll_for_initiative.badges.push(BadgeView {
                    token,
                    order,
                    name,
                    total,
                    age_s: 0.0,
                });
            }
            Step::TrackerReveal(token) => {
                if let Some(tracker) = &mut self.hud.roll_for_initiative.tracker
                    && !tracker.contains(&token)
                {
                    tracker.push(token);
                }
            }
            Step::InitiativeEnd => self.hud.roll_for_initiative = InitiativeShow::default(),
            Step::Sting { token, index, last } => {
                let kind = if last {
                    SoundKind::InitiativeFinal
                } else {
                    SoundKind::InitiativeStop
                };
                // Each stop sounds a little higher; the last, heavy one a little lower.
                let rate = if last { 0.8 } else { 0.9 + 0.13 * index as f32 };
                self.emit_at_rate(kind, &token, rate);
            }
            Step::Target(target) => self.target = target,
            Step::Projectile {
                from,
                to,
                hits,
                flight,
            } => self.launch(&from, &to, hits, flight),
        }
    }

    /// Looses an arrow from `from` toward `to`. A miss flies on past the target and sticks in
    /// the ground behind it.
    fn launch(&mut self, from: &TokenId, to: &TokenId, hits: bool, flight: Duration) {
        let (Some(archer), Some(mark)) = (self.token_index(from), self.token_index(to)) else {
            return;
        };
        let (archer, mark) = (&self.tokens[archer], &self.tokens[mark]);
        let heading = flat_direction(archer.position, mark.position);
        let bow = archer.position
            + heading * 0.3
            + Vec3::new(0.0, archer.token.clamped_height() * 0.62, 0.0);
        let chest = mark.position + Vec3::new(0.0, mark.token.clamped_height() * 0.55, 0.0);
        let end = if hits {
            chest
        } else {
            mark.position + heading * 2.4 + Vec3::new(0.0, 0.12, 0.0)
        };
        self.projectiles.push(Projectile {
            from: bow,
            to: end,
            hits,
            flight_s: flight.as_secs_f32().max(0.05),
            age_s: 0.0,
        });
    }

    /// Walks the token from where it stands to `cell`. Its formation cell does not change: the
    /// token's `cell` stays home so the strike can send it back.
    fn move_to(&mut self, id: &TokenId, cell: Cell, per_step: Duration) {
        let Some(index) = self.token_index(id) else {
            return;
        };
        let reduced = self.reduced_motion;
        let grid = &self.grid;
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
            landed: 0,
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
        if matches!(anim, Anim::Draw { .. }) && self.reduced_motion {
            return;
        }
        let here = self.tokens[index].position;
        let other = match &anim {
            Anim::Lunge { toward: other }
            | Anim::Draw { toward: other }
            | Anim::Recoil { from: other }
            | Anim::Dodge { from: other } => self
                .token_index(other)
                .map(|other| self.tokens[other].position),
            Anim::Fall | Anim::Heal | Anim::Ready { .. } => None,
        };
        let direction = match (&anim, other) {
            (Anim::Lunge { .. } | Anim::Draw { .. }, Some(other)) => flat_direction(here, other),
            (Anim::Ready { .. }, _) => {
                // Facing the other side: heroes stand near the camera, enemies far.
                if self.tokens[index].token.side == Side::Enemy {
                    Vec3::new(0.0, 0.0, 1.0)
                } else {
                    Vec3::new(0.0, 0.0, -1.0)
                }
            }
            (Anim::Recoil { .. } | Anim::Dodge { .. }, Some(other)) => flat_direction(other, here),
            _ => Vec3::new(0.0, 0.0, -1.0),
        };
        self.tokens[index].anim = Some(ActiveAnim {
            anim,
            elapsed_s: 0.0,
            direction,
            landed: false,
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

    /// Replaces the painted ground and backdrop and drops the splat layer: the fallback when a
    /// scan cannot be drawn.
    pub fn replace_painted_layers(
        &mut self,
        ground: super::scene_file::GroundLayer,
        backdrop: super::scene_file::BackdropLayer,
    ) {
        self.scene.ground = ground;
        self.scene.backdrop = backdrop;
        self.scene.splat = None;
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
            projectiles: self
                .projectiles
                .iter()
                .map(|projectile| projectile_view(projectile, self.reduced_motion))
                .collect(),
            layout: self.layout(),
            grid: &self.grid,
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
            reduced_motion: self.reduced_motion,
            restrained: self.restrained,
        }
    }

    /// What the figure shows: the server's statuses (applied with the blow that caused them)
    /// plus the two states that follow from the hit points already on screen. Nothing else is
    /// invented here: bloodied is at or below half, unconscious is lying at zero.
    fn display_statuses(&self, runtime: &Runtime) -> StatusSet {
        let mut set = runtime.server_status;
        if let Some(hp) = runtime.shown_hp
            && hp.current > 0
            && hp.current * 2 <= hp.maximum
        {
            set = set.with(Status::Bloodied);
        }
        if runtime.downed {
            set = set.with(Status::Unconscious);
        }
        set
    }

    fn cover_pip(&self, runtime: &Runtime, statuses: StatusSet) -> Option<CoverPip> {
        if runtime.walk.is_some() || runtime.downed {
            return None;
        }
        let cell = self.grid.cell_at(runtime.position)?;
        if let Some((beside, level)) = self.grid.cover_beside(cell) {
            let delta = self.grid.cell_centre(beside) - self.grid.cell_centre(cell);
            let toward = flat_direction(Vec3::ZERO, delta);
            return Some(CoverPip {
                toward: [toward.x, toward.z],
                level,
            });
        }
        // The server says it is in cover but the layout shows none beside it: face the far side.
        statuses.contains(Status::Cover).then_some(CoverPip {
            toward: [0.0, -1.0],
            level: Cover::Half,
        })
    }

    fn figure(&self, runtime: &Runtime) -> FigureView {
        let mut position = runtime.position;
        let mut lean = 0.0;
        let mut red_flash = 0.0;
        let mut heal_glow = 0.0;
        let clock = self.clock_s;
        let phase = runtime.phase;
        let unit = phase / std::f32::consts::TAU;
        let reach = self.grid.cell_m() * 0.5;
        let statuses = self.display_statuses(runtime);
        let pose = status::pose_for(statuses, self.reduced_motion);
        let mut slump = pose.slump;
        let mut busy = runtime.walk.is_some();
        let mut falling = false;
        if let Some(active) = &runtime.anim {
            busy = true;
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
                    falling = true;
                    slump = slump.max(fall_slump(progress));
                    red_flash = (1.0 - progress) * 0.6;
                }
                Anim::Heal => {
                    heal_glow = arc;
                    position.y += 0.05 * arc;
                }
                Anim::Ready { style } => match style {
                    ReadyStyle::Brandish => {
                        // The weapon comes up and the body settles back down.
                        position.y += 0.2 * arc;
                        lean = -0.04 * arc;
                    }
                    ReadyStyle::Snarl => {
                        position = position - active.direction * (0.22 * arc);
                        lean = 0.12 * arc;
                    }
                    ReadyStyle::Nock => {
                        lean = -0.1 * arc;
                        position = position - active.direction * (0.08 * arc);
                    }
                },
                Anim::Draw { .. } => {
                    // Lean back while the string comes to the ear, then snap forward on release.
                    let release = director::DRAW_RELEASE.as_secs_f32() / duration;
                    let pull = if progress < release {
                        ease_in_out(progress / release)
                    } else {
                        -0.35 * (1.0 - (progress - release) / (1.0 - release))
                    };
                    position = position - active.direction * (0.14 * pull);
                    lean = -0.17 * pull * active.direction.x.signum();
                }
            }
        }
        if runtime.downed && !falling {
            slump = 1.0;
        }
        if let Some(walk) = &runtime.walk {
            position = walk_position(walk);
        }
        let resting = !busy && !runtime.downed && !falling;
        if resting && pose.idle > 0.0 && !self.reduced_motion {
            // Weight shifts: two slow sways at rates that differ per figure, so a line of
            // figures never moves in step.
            lean += pose.idle
                * (0.014 * (clock * (0.5 + 0.18 * unit) + phase).sin()
                    + 0.010 * (clock * (0.23 + 0.11 * unit) + phase * 2.1).sin());
        }
        if pose.flinch > 0.0
            && let Some(nearest) = self.nearest_opponent(runtime)
        {
            lean -= pose.flinch * (nearest.x - position.x).signum();
        }
        lean += 0.06 * pose.wobble * (clock * 8.0 + phase).sin();
        position.x += 0.012 * pose.shiver * (clock * 60.0 + phase).sin();
        let afterimage = if pose.afterimage && resting {
            (clock * 1.1 + phase * 3.0).sin().max(0.0).powi(6)
        } else {
            0.0
        };
        let heartbeat = if pose.heartbeat || (runtime.downed && statuses.contains(Status::Dying)) {
            (clock * 7.5).sin().max(0.0).powi(8)
        } else {
            0.0
        };
        // A figure laid low (down, falling, or unconscious at 1 hit point) is dimmed only a little,
        // and less still when it wears its health variant, so it reads as a fallen person and not
        // a dark heap against a night scan.
        let unconscious =
            statuses.contains(Status::Unconscious) || statuses.contains(Status::Dying);
        let dim = if runtime.downed || falling || unconscious {
            if runtime.shown_variant.is_some() {
                LAID_LOW_DIM_WITH_VARIANT
            } else {
                LAID_LOW_DIM
            }
        } else {
            pose.dim
        };
        let breathing = if runtime.downed || pose.idle == 0.0 {
            0.0
        } else {
            0.012 * pose.breath * pose.idle.max(0.4)
        };
        let rest = self.grid.cell_centre(runtime.token.cell);
        let lift = ((position - rest).length() / reach.max(0.01)).min(2.0);
        let down = runtime.downed
            || statuses.contains(Status::Unconscious)
            || statuses.contains(Status::Dying);
        let tone = runtime
            .shown_hp
            .and_then(|hp| health::tone_of(hp.current, hp.maximum, down));
        let position = self.shaken(runtime, tone, position);
        FigureView {
            id: runtime.token.id.clone(),
            side: runtime.token.side,
            name: runtime.token.name.clone(),
            position,
            height_m: runtime.token.clamped_height(),
            lean,
            alpha: pose.alpha,
            red_flash,
            heal_glow,
            breath: 1.0 + breathing * (clock * (2.0 + 0.6 * unit) + phase).sin(),
            active: self.hud.active.as_ref() == Some(&runtime.token.id),
            targeted: self.target.as_ref() == Some(&runtime.token.id),
            slump,
            thud: (runtime.thud_s / THUD_SECONDS).clamp(0.0, 1.0),
            downed: runtime.downed,
            dim,
            statuses,
            pose,
            cover: self.cover_pip(runtime, statuses),
            afterimage,
            heartbeat,
            phase,
            mirror: runtime.facing_left,
            figure: runtime.token.figure.clone(),
            portrait: runtime.token.portrait.clone(),
            hit_points: runtime.shown_hp,
            variant: runtime.shown_variant.clone(),
            tone,
            lift,
        }
    }

    /// `position` moved by the figure's bodily reaction: the shake of a blow it just took, and
    /// at Critical the slow heartbeat tremor. Sideways is screen-sideways; downward is a stomp
    /// into the ground. Presentation only, and the one place a figure's reaction is applied.
    fn shaken(&self, runtime: &Runtime, tone: Option<Tone>, position: Vec3) -> Vec3 {
        let mut side = 0.0;
        let mut down = 0.0;
        if let Some((reaction, age)) = &runtime.reaction {
            let (dx, dy) = reaction.offset(*age);
            side += dx;
            down += dy;
        }
        if tone == Some(Tone::Critical)
            && let Some(tremor) = health::tremor()
        {
            let (dx, dy) = tremor.offset(self.clock_s + runtime.phase);
            side += dx;
            down += dy;
        }
        if side == 0.0 && down == 0.0 {
            return position;
        }
        let pose = self.rig.pose();
        let right = (pose.target - pose.eye()).normalized().cross(Vec3::UP);
        let flat = Vec3::new(right.x, 0.0, right.z).normalized();
        let mut moved = position + flat * side;
        moved.y -= down;
        moved
    }

    /// Starts `reaction` on the figure `id`, replacing any in progress. The one entry point for
    /// a hit reaction: a director that sequences its own can call it with its own timing.
    pub fn react(&mut self, id: &TokenId, reaction: Reaction) {
        if let Some(index) = self.token_index(id) {
            self.tokens[index].reaction = Some((reaction, 0.0));
        }
    }

    fn nearest_opponent(&self, runtime: &Runtime) -> Option<Vec3> {
        self.tokens
            .iter()
            .filter(|other| other.token.side != runtime.token.side && !other.downed)
            .map(|other| other.position)
            .min_by(|a, b| {
                (*a - runtime.position)
                    .length()
                    .total_cmp(&(*b - runtime.position).length())
            })
    }
}

/// A stable phase in `0..TAU` from a token id (FNV-1a), so each figure idles out of step.
fn phase_of(id: &TokenId) -> f32 {
    let hash = id.0.bytes().fold(0x811c_9dc5_u32, |state, byte| {
        (state ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    (hash % 6283) as f32 / 1000.0
}

/// Slump during a fall: drop to a knee first, then crumple to the ground after the landing.
fn fall_slump(progress: f32) -> f32 {
    let landing = FALL_LANDS.as_secs_f32() / director::FALL.as_secs_f32();
    if progress < landing {
        FALL_KNEEL * ease_in_out(progress / landing)
    } else {
        FALL_KNEEL + (1.0 - FALL_KNEEL) * ease_in_out((progress - landing) / (1.0 - landing))
    }
}

fn arrow_at(projectile: &Projectile, t: f32) -> Vec3 {
    let travel = projectile.to - projectile.from;
    let height = (0.2 + 0.05 * travel.length()) * 4.0 * t * (1.0 - t);
    projectile.from + travel * t + Vec3::new(0.0, height, 0.0)
}

fn projectile_view(projectile: &Projectile, reduced_motion: bool) -> ProjectileView {
    let t = (projectile.age_s / projectile.flight_s).clamp(0.0, 1.0);
    let head = arrow_at(projectile, t);
    let tangent = (head - arrow_at(projectile, (t - 0.03).max(0.0))).normalized();
    let tangent = if tangent == Vec3::ZERO {
        (projectile.to - projectile.from).normalized()
    } else {
        tangent
    };
    let tail = head - tangent * 0.85;
    let trail = if reduced_motion {
        tail
    } else {
        arrow_at(projectile, (t - 0.28).max(0.0))
    };
    let stuck_for = (projectile.age_s - projectile.flight_s).max(0.0);
    let alpha = if projectile.hits {
        1.0
    } else {
        1.0 - ((stuck_for - (ARROW_STICK_SECONDS - 0.3)) / 0.3).clamp(0.0, 1.0)
    };
    ProjectileView {
        head,
        tail,
        trail: if stuck_for > 0.0 { tail } else { trail },
        alpha,
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

/// Advances one figure's walk and animation. Returns how many footfalls landed this tick.
impl Runtime {
    /// Shows these hit points. A blow that took some starts the figure's reaction (sized by the
    /// share of the bar it took, bigger when critical, a slump when it dropped the figure), by
    /// whichever path the new hit points arrive; the health variant of the picture changes with
    /// them.
    fn show_hp(&mut self, hp: Option<HitPoints>) {
        if let (Some(old), Some(new)) = (self.shown_hp, hp)
            && new.current < old.current
            && let Some(reaction) = health::reaction_for(
                old.current - new.current,
                new.maximum,
                self.critical_next,
                new.current == 0,
            )
        {
            self.reaction = Some((reaction, 0.0));
        }
        if hp.is_some_and(|hp| hp.current != self.shown_hp.map_or(u32::MAX, |old| old.current)) {
            self.critical_next = false;
        }
        self.shown_hp = hp;
        self.shown_variant = health::variant_for(&self.token.name);
    }
}

fn advance_runtime(runtime: &mut Runtime, dt: f32) -> usize {
    let mut footfalls = 0;
    if let Some(walk) = &mut runtime.walk {
        walk.elapsed_s += dt;
        let landed = ((walk.elapsed_s / walk.per_step_s).floor() as usize).min(walk.cells.len());
        footfalls = landed.saturating_sub(walk.landed);
        walk.landed = landed;
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
    footfalls
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{Cover, SoundKind, Status};
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
            attack: crate::battle::model::AttackStyle::Melee,
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
        assert!(hero.lean.abs() < 0.03 && hero.alpha == 1.0);
    }

    #[test]
    fn figures_stand_on_the_flat_play_plane_fitted_to_the_scanned_ground() {
        let mut scene = load_embedded("salt-quay").unwrap().unwrap();
        // A gently sloping field, 10 cm per metre along x, covering the grid.
        let origin = [scene.grid.origin()[0] - 1.0, scene.grid.origin()[1] - 1.0];
        let heights: Vec<f32> = (0..40)
            .flat_map(|_| (0..40).map(|col| col as f32 * 0.1))
            .collect();
        let field = super::super::terrain::HeightField::new(origin, 1.0, 40, 40, heights).unwrap();
        scene.grid = scene.grid.clone().with_terrain(field);
        let mut stage = Stage::new(scene, false);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(1, 4), 10),
            token("goblin", Side::Enemy, Cell::new(6, 4), 8),
        ]);
        let frame = stage.frame();
        let heights: Vec<(f32, f32)> = frame
            .figures
            .iter()
            .map(|figure| (figure.position.x, figure.position.y))
            .collect();
        assert_eq!(heights.len(), 2);
        let plane = stage.scene().grid.play_plane_y();
        for (_, y) in heights {
            assert!(
                (y - plane).abs() < 1e-5,
                "feet at {y} on the flat play plane {plane}"
            );
        }
        // The sloping field is only measured against the plane, never followed.
        assert!(stage.scene().grid.plane_deviation_m() > 0.3);
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
            enemy.downed && enemy.slump > 0.99 && enemy.alpha == 1.0 && enemy.dim > 0.3,
            "enemy lies on the ground, dimmed but solid: {enemy:?}"
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
        assert!(hero.lean.abs() < 0.03);
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
        assert!(enemy.downed && enemy.slump > 0.99);
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

    fn sounds_after(stage: &mut Stage, seconds: f32) -> Vec<(f32, SoundKind, String)> {
        let mut heard = Vec::new();
        for tick in 0..(seconds / 0.05) as u32 {
            stage.update(Duration::from_millis(50));
            for event in stage.take_sounds() {
                heard.push((tick as f32 * 0.05, event.kind, event.token.0));
            }
        }
        heard
    }

    #[test]
    fn one_footstep_sounds_per_cell_walked_there_and_back() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        let heard = sounds_after(&mut stage, 9.0);
        let steps = heard
            .iter()
            .filter(|(_, kind, who)| *kind == SoundKind::Footstep && who == "hero")
            .count();
        let out = stage
            .grid()
            .path(Cell::new(3, 4), Cell::new(3, 2))
            .map_or(0, |path| path.len() - 1);
        assert_eq!(steps, out * 2, "out and back, one per cell");
        let order: Vec<SoundKind> = heard.iter().map(|(_, kind, _)| *kind).collect();
        let first_swing = order
            .iter()
            .position(|kind| *kind == SoundKind::SwordSwing)
            .unwrap();
        let last_out_step = heard
            .iter()
            .take(first_swing)
            .filter(|(_, kind, _)| *kind == SoundKind::Footstep)
            .count();
        assert_eq!(last_out_step, out, "the steps in come before the swing");
    }

    #[test]
    fn reduced_motion_teleports_and_so_makes_no_footsteps() {
        let mut stage = stage(true);
        stage.stage(killing_blow()).unwrap();
        let heard = sounds_after(&mut stage, 20.0);
        assert!(
            heard
                .iter()
                .all(|(_, kind, _)| *kind != SoundKind::Footstep)
        );
        assert!(
            heard
                .iter()
                .any(|(_, kind, _)| *kind == SoundKind::BladeHit)
        );
    }

    #[test]
    fn voices_follow_the_token_and_the_surface_the_scene() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        let mut seen = Vec::new();
        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
            seen.extend(stage.take_sounds());
        }
        let effort = seen
            .iter()
            .find(|event| event.kind == SoundKind::Effort)
            .unwrap();
        assert_eq!(effort.voice, crate::battle::VoiceSet::HeroMale);
        let step = seen
            .iter()
            .find(|event| event.kind == SoundKind::Footstep)
            .unwrap();
        assert_eq!(step.surface, stage.scene().surface);
    }

    #[test]
    fn the_sound_queue_is_bounded_and_counts_what_it_dropped() {
        let mut stage = stage(false);
        for _ in 0..(MAX_PENDING_SOUNDS + 5) {
            stage.emit(SoundKind::HealBreath, &TokenId::new("hero"));
        }
        assert_eq!(stage.take_sounds().len(), MAX_PENDING_SOUNDS);
        assert_eq!(stage.dropped_sounds(), 5);
    }

    fn archer_stage() -> Stage {
        let mut stage = stage(false);
        let mut archer = token("hero", Side::Hero, Cell::new(3, 4), 12);
        archer.attack = crate::battle::model::AttackStyle::Ranged;
        stage.set_tokens(vec![
            archer,
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        stage
    }

    #[test]
    fn an_arrow_flies_an_arc_from_the_bow_to_the_target_and_vanishes_on_a_hit() {
        let mut stage = archer_stage();
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Hit {
                    damage: 3,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(2),
            })
            .unwrap();
        let mut heights = Vec::new();
        let mut in_flight = 0;
        for _ in 0..200 {
            stage.update(Duration::from_millis(20));
            let frame = stage.frame();
            if let Some(arrow) = frame.projectiles.first() {
                in_flight += 1;
                heights.push(arrow.head.y);
                assert!(arrow.tail != arrow.head && arrow.alpha > 0.0);
            }
        }
        let flight_ms = in_flight * 20;
        assert!(
            (340..=500).contains(&flight_ms),
            "{flight_ms} ms in the air"
        );
        let peak = heights.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            peak > heights[0] && peak > *heights.last().unwrap(),
            "an arc, not a line"
        );
        assert!(stage.frame().projectiles.is_empty());
        let hero = figure(&stage, "hero");
        assert!((hero.position - stage.grid().cell_centre(Cell::new(3, 4))).length() < 0.01);
    }

    #[test]
    fn a_missed_arrow_flies_past_and_sticks_in_the_ground_before_it_goes() {
        let mut stage = archer_stage();
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        let target = stage.grid().cell_centre(Cell::new(4, 1));
        let mut stuck = None;
        for _ in 0..300 {
            stage.update(Duration::from_millis(20));
            if let Some(arrow) = stage.frame().projectiles.first()
                && arrow.trail == arrow.tail
            {
                stuck = Some(*arrow);
            }
        }
        let arrow = stuck.expect("the arrow stuck");
        assert!(arrow.head.y < 0.3, "in the ground");
        assert!(
            arrow.head.z < target.z,
            "past the target, away from the archer"
        );
        assert!(stage.frame().projectiles.is_empty(), "then gone");
    }

    #[test]
    fn the_downed_state_kneels_then_lies_on_the_ground_with_a_landing_squash() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        let mut slumps = Vec::new();
        let mut thud_peak = 0.0f32;
        for _ in 0..400 {
            stage.update(Duration::from_millis(25));
            let enemy = figure(&stage, "enemy");
            slumps.push(enemy.slump);
            thud_peak = thud_peak.max(enemy.thud);
            assert_eq!(
                enemy.alpha, 1.0,
                "solid on the ground, never fading in the air"
            );
        }
        assert!(
            slumps.windows(2).all(|pair| pair[1] >= pair[0] - 1e-4),
            "monotonic"
        );
        assert!(
            slumps.iter().any(|slump| (0.3..0.6).contains(slump)),
            "passes the knee"
        );
        assert!(thud_peak > 0.9, "squashes on landing");
        let enemy = figure(&stage, "enemy");
        assert!((enemy.slump, enemy.thud) == (1.0, 0.0) && enemy.downed);
        assert!(enemy.statuses.contains(Status::Unconscious));
    }

    #[test]
    fn a_downed_figure_that_is_healed_stands_again() {
        let mut stage = stage(false);
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 20.0);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 6),
        ]);
        run(&mut stage, 0.2);
        let enemy = figure(&stage, "enemy");
        assert!(!enemy.downed && enemy.slump < 0.01);
    }

    #[test]
    fn idle_figures_sway_out_of_step_and_over_time() {
        let mut stage = stage(false);
        run(&mut stage, 1.0);
        let at_one: Vec<f32> = ["hero", "ally", "enemy"]
            .iter()
            .map(|id| figure(&stage, id).lean)
            .collect();
        assert!(at_one[0] != at_one[1] && at_one[1] != at_one[2]);
        run(&mut stage, 3.0);
        assert!(figure(&stage, "hero").lean != at_one[0]);
        let reduced = {
            let mut calm = self::stage(true);
            run(&mut calm, 2.0);
            figure(&calm, "hero").lean
        };
        assert_eq!(reduced, 0.0, "no idle sway under reduced motion");
    }

    #[test]
    fn the_target_ring_shows_during_the_focus_and_clears_at_contact() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        run(&mut stage, 0.5);
        assert!(figure(&stage, "enemy").targeted && !figure(&stage, "hero").targeted);
        run(&mut stage, 6.0);
        assert!(!figure(&stage, "enemy").targeted);
    }

    #[test]
    fn bloodied_follows_the_hit_points_on_screen_and_unconscious_the_fall() {
        let mut stage = stage(false);
        // The enemy has 5 of 12: already bloodied at rest (at or below half).
        assert!(figure(&stage, "enemy").statuses.contains(Status::Bloodied));
        assert!(!figure(&stage, "hero").statuses.contains(Status::Bloodied));
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 0.6);
        assert!(
            !figure(&stage, "enemy")
                .statuses
                .contains(Status::Unconscious)
        );
        run(&mut stage, 5.0);
        let enemy = figure(&stage, "enemy");
        assert!(enemy.statuses.contains(Status::Unconscious));
        assert!(
            !enemy.statuses.contains(Status::Bloodied),
            "zero is not bloodied"
        );
    }

    #[test]
    fn a_server_status_waits_for_the_blow_that_causes_it() {
        let mut stage = stage(false);
        let enemy_id = TokenId::new("enemy");
        stage.hold_hit_points([&enemy_id]);
        let mut poisoned = token("enemy", Side::Enemy, Cell::new(4, 1), 4);
        poisoned.statuses = vec![Status::Poisoned];
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            poisoned,
        ]);
        assert!(!figure(&stage, "enemy").statuses.contains(Status::Poisoned));
        run(&mut stage, HP_HOLD_SECONDS + 1.0);
        assert!(figure(&stage, "enemy").statuses.contains(Status::Poisoned));
    }

    #[test]
    fn statuses_shape_the_pose_and_prone_with_poison_shows_both() {
        let mut stage = stage(false);
        let mut ally = token("ally", Side::Hero, Cell::new(5, 4), 12);
        ally.statuses = vec![Status::Prone, Status::Poisoned];
        let mut hero = token("hero", Side::Hero, Cell::new(3, 4), 12);
        hero.statuses = vec![Status::Petrified];
        stage.set_tokens(vec![
            hero,
            ally,
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        run(&mut stage, 1.0);
        let ally = figure(&stage, "ally");
        assert_eq!(ally.slump, 1.0);
        assert!(ally.statuses.contains(Status::Poisoned) && ally.dim == 0.0);
        assert!(!ally.downed, "prone is conscious");
        let stone = figure(&stage, "hero");
        assert_eq!(stone.lean, 0.0, "stone does not sway");
        assert_eq!(stone.breath, 1.0);
    }

    #[test]
    fn a_frightened_figure_leans_away_from_the_nearest_opponent() {
        let mut stage = stage(false);
        let mut hero = token("hero", Side::Hero, Cell::new(1, 4), 12);
        hero.statuses = vec![Status::Frightened];
        stage.set_tokens(vec![
            hero,
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(6, 1), 5),
        ]);
        let enemy_x = stage.grid().cell_centre(Cell::new(6, 1)).x;
        let hero_x = stage.grid().cell_centre(Cell::new(1, 4)).x;
        assert!(enemy_x > hero_x);
        let mut leans = Vec::new();
        for _ in 0..60 {
            stage.update(Duration::from_millis(50));
            leans.push(figure(&stage, "hero").lean);
        }
        let mean = leans.iter().sum::<f32>() / leans.len() as f32;
        assert!(mean < -0.1, "leans away (left): {mean}");
    }

    fn layout_stage() -> Stage {
        let mut scene = load_embedded("salt-quay").unwrap().unwrap();
        scene.layouts = vec![crate::battle::Layout {
            id: "wall".to_owned(),
            label: "Wall".to_owned(),
            obstacles: vec![crate::battle::Obstacle {
                id: "crates".to_owned(),
                kind: "crate".to_owned(),
                cells: (1..6).map(|col| Cell::new(col, 3)).collect(),
                blocks_movement: true,
                cover: Cover::Half,
                sprite: "prop-crates".to_owned(),
                height_m: 1.1,
            }],
            difficult: vec![],
            heroes: (0..4).map(|col| Cell::new(col + 1, 5)).collect(),
            enemies: (0..4).map(|col| Cell::new(col + 1, 0)).collect(),
        }];
        let mut stage = Stage::new(scene, false);
        assert!(stage.set_layout("wall"));
        assert!(!stage.set_layout("missing"));
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 5), 12),
            token("enemy", Side::Enemy, Cell::new(3, 0), 5),
        ]);
        stage
    }

    #[test]
    fn a_figure_walks_around_an_obstacle_and_never_stands_on_a_blocked_cell() {
        let mut stage = layout_stage();
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        let mut cells = std::collections::BTreeSet::new();
        for _ in 0..600 {
            stage.update(Duration::from_millis(25));
            let hero = figure(&stage, "hero");
            if let Some(cell) = stage.grid().cell_at(hero.position) {
                assert!(
                    stage.grid().is_walkable(cell),
                    "stood in a crate at {cell:?}"
                );
                cells.insert(cell);
            }
        }
        assert!(
            cells.iter().any(|cell| cell.col == 0 || cell.col == 6),
            "went round the end"
        );
        assert!(cells.len() > 6);
        assert!(!stage.is_busy());
    }

    #[test]
    fn figures_beside_cover_show_a_pip_toward_it() {
        let stage = layout_stage();
        let hero = figure(&stage, "hero");
        assert!(hero.cover.is_none(), "row 5 is two cells from the crates");
        let mut stage = layout_stage();
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("enemy", Side::Enemy, Cell::new(3, 2), 5),
        ]);
        let hero = figure(&stage, "hero");
        let pip = hero.cover.expect("beside the crates");
        assert_eq!(pip.level, Cover::Half);
        assert!(
            pip.toward[1] < -0.9,
            "toward the row of crates, away from the camera"
        );
        assert!(figure(&stage, "enemy").cover.expect("other side").toward[1] > 0.9);
    }

    #[test]
    fn spawn_cells_seat_the_roster_when_a_layout_is_chosen() {
        use crate::battle::{CombatFeed, CombatSnapshot, Member};
        let mut stage = layout_stage();
        let mut feed = CombatFeed::new(crate::battle::FeedMode::HoldForCues);
        let member = |id: &str, side| Member {
            id: id.to_owned(),
            name: id.to_owned(),
            side,
            hit_points: 12,
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: crate::battle::model::AttackStyle::Melee,
            cell: None,
            statuses: Vec::new(),
        };
        feed.apply(
            &mut stage,
            &CombatSnapshot {
                members: vec![
                    member("a", Side::Hero),
                    member("b", Side::Hero),
                    member("e", Side::Enemy),
                ],
                active: None,
                round: 1,
                outcomes: vec![],
                initiative: vec![],
                begun_live: false,
            },
        )
        .unwrap();
        let cells: Vec<(String, Cell)> = stage.tokens().map(|t| (t.id.0.clone(), t.cell)).collect();
        for (id, cell) in &cells {
            let layout = stage.layout().unwrap();
            let spawns = if id == "e" {
                &layout.enemies
            } else {
                &layout.heroes
            };
            assert!(spawns.contains(cell), "{id} at {cell:?}");
        }
    }

    #[test]
    fn a_server_supplied_cell_replaces_the_spawn() {
        use crate::battle::{CombatFeed, CombatSnapshot, Member};
        let mut stage = layout_stage();
        let mut feed = CombatFeed::new(crate::battle::FeedMode::HoldForCues);
        let member = |id: &str, side, cell| Member {
            id: id.to_owned(),
            name: id.to_owned(),
            side,
            hit_points: 12,
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: crate::battle::model::AttackStyle::Melee,
            cell,
            statuses: Vec::new(),
        };
        feed.apply(
            &mut stage,
            &CombatSnapshot {
                members: vec![
                    member("a", Side::Hero, Some(Cell::new(6, 4))),
                    member("e", Side::Enemy, None),
                ],
                active: None,
                round: 1,
                outcomes: vec![],
                initiative: vec![],
                begun_live: false,
            },
        )
        .unwrap();
        assert!(
            stage
                .tokens()
                .any(|t| t.id.0 == "a" && t.cell == Cell::new(6, 4))
        );
    }

    fn rolls_for(order: &[&str]) -> Vec<crate::battle::InitiativeRoll> {
        order
            .iter()
            .enumerate()
            .map(|(index, id)| crate::battle::InitiativeRoll {
                token: TokenId::new(*id),
                die: 18 - index as u32 * 4,
                modifier: 2,
                total: 20 - index as i32 * 4,
            })
            .collect()
    }

    #[test]
    fn the_roll_call_fills_the_tracker_in_the_servers_order_with_badges_1st_2nd_3rd() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Initiative {
                rolls: rolls_for(&["enemy", "hero", "ally"]),
            })
            .unwrap();
        assert!(stage.is_busy());
        let mut tracker_sizes = Vec::new();
        let mut seen_badges: Vec<(u32, String)> = Vec::new();
        let mut banners = Vec::new();
        for _ in 0..900 {
            stage.update(Duration::from_millis(20));
            let show = &stage.hud().roll_for_initiative;
            if let Some(tracker) = &show.tracker
                && tracker_sizes.last() != Some(&tracker.len())
            {
                tracker_sizes.push(tracker.len());
                let expected = ["enemy", "hero", "ally"];
                assert!(
                    tracker
                        .iter()
                        .zip(expected)
                        .all(|(have, want)| have.0 == want)
                );
            }
            for badge in &show.badges {
                if !seen_badges.iter().any(|(order, _)| *order == badge.order) {
                    seen_badges.push((badge.order, badge.name.clone()));
                }
            }
            if let Some(banner) = &show.banner
                && banners.last() != Some(banner)
            {
                banners.push(banner.clone());
            }
        }
        assert_eq!(tracker_sizes, vec![0, 1, 2, 3]);
        assert_eq!(
            seen_badges,
            vec![
                (1, "ENEMY".to_owned()),
                (2, "HERO".to_owned()),
                (3, "ALLY".to_owned())
            ]
        );
        assert_eq!(
            banners,
            vec!["Roll for initiative".to_owned(), "Round 1".to_owned()]
        );
        assert!(!stage.is_busy());
        let show = &stage.hud().roll_for_initiative;
        assert!(show.tracker.is_none() && show.badges.is_empty() && show.dice.is_empty());
        assert_eq!(
            stage.hud().active,
            Some(TokenId::new("enemy")),
            "the first turn follows"
        );
    }

    #[test]
    fn each_die_tumbles_then_lands_on_its_total() {
        let mut stage = stage(false);
        stage
            .stage(Beat::Initiative {
                rolls: rolls_for(&["hero", "ally", "enemy"]),
            })
            .unwrap();
        let mut tumbling = false;
        let mut landed_after_tumble = false;
        for _ in 0..300 {
            stage.update(Duration::from_millis(20));
            for die in &stage.hud().roll_for_initiative.dice {
                if !die.landed {
                    tumbling = true;
                } else if tumbling && die.total == die.face as i32 + die.modifier {
                    landed_after_tumble = true;
                }
            }
        }
        assert!(tumbling && landed_after_tumble);
    }

    #[test]
    fn the_camera_whips_to_each_stop_and_cuts_with_a_fade_under_reduced_motion() {
        let mut calm = stage(true);
        calm.stage(Beat::Initiative {
            rolls: rolls_for(&["hero", "enemy", "ally"]),
        })
        .unwrap();
        let mut lively = stage(false);
        lively
            .stage(Beat::Initiative {
                rolls: rolls_for(&["hero", "enemy", "ally"]),
            })
            .unwrap();
        let (mut calm_panned, mut lively_overshoot) = (false, false);
        let mut closest = f32::MAX;
        for _ in 0..900 {
            calm.update(Duration::from_millis(20));
            lively.update(Duration::from_millis(20));
            calm_panned |= calm.rig.is_panning();
            // A whip travels farther than it needs to and comes back: the lens punches in.
            closest = closest.min(lively.camera_pose().fov_deg);
            lively_overshoot |= lively.rig.is_panning();
        }
        assert!(!calm_panned, "reduced motion never pans: it cuts");
        assert!(lively_overshoot);
        let victory = lively.scene().cameras[&ShotName::Victory].fov_deg;
        assert!(
            closest < victory * 0.97,
            "zoom punch on arrival: {closest} vs {victory}"
        );
        assert!(!calm.is_busy() && !lively.is_busy());
    }

    #[test]
    fn a_newer_view_cuts_the_sequence_short_and_goes_to_the_current_turn() {
        let mut stage = stage(false);
        stage.set_active(Some(TokenId::new("hero")));
        stage
            .stage(Beat::Initiative {
                rolls: rolls_for(&["hero", "enemy", "ally"]),
            })
            .unwrap();
        run(&mut stage, 3.0);
        assert!(stage.is_rolling_initiative());
        stage.skip_initiative();
        assert!(!stage.is_rolling_initiative() && !stage.is_busy());
        let show = &stage.hud().roll_for_initiative;
        assert!(show.tracker.is_none() && show.badges.is_empty() && show.banner.is_none());
        assert!(!stage.rig.is_panning());
    }

    fn feed_snapshot(
        begun_live: bool,
        outcomes: usize,
        round: u32,
    ) -> crate::battle::CombatSnapshot {
        use crate::battle::{InitiativeEntry, Member, Outcome};
        let member = |id: &str, side| Member {
            id: id.to_owned(),
            name: id.to_uppercase(),
            side,
            hit_points: 12,
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: crate::battle::model::AttackStyle::Melee,
            cell: None,
            statuses: Vec::new(),
        };
        crate::battle::CombatSnapshot {
            members: vec![
                member("a", Side::Hero),
                member("b", Side::Hero),
                member("e", Side::Enemy),
            ],
            active: Some("e".to_owned()),
            round,
            outcomes: (0..outcomes)
                .map(|_| Outcome {
                    actor: "e".to_owned(),
                    target: "a".to_owned(),
                    hit: false,
                    critical: false,
                    damage: 0,
                    target_hit_points_after: 12,
                    downed: false,
                })
                .collect(),
            initiative: ["e", "a", "b"]
                .iter()
                .enumerate()
                .map(|(index, id)| InitiativeEntry {
                    id: (*id).to_owned(),
                    die: 18 - index as u32 * 5,
                    modifier: 1,
                    total: 19 - index as i32 * 5,
                })
                .collect(),
            begun_live,
        }
    }

    #[test]
    fn the_initiative_is_staged_for_a_fight_the_display_watched_begin() {
        use crate::battle::{CombatFeed, FeedMode};
        let mut stage = stage(false);
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(&mut stage, &feed_snapshot(true, 0, 1)).unwrap();
        assert!(stage.is_busy());
        run(&mut stage, 0.2);
        assert!(stage.is_rolling_initiative());
        assert_eq!(
            stage
                .hud()
                .initiative
                .iter()
                .map(|id| id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["e", "a", "b"],
            "the tracker order is the server's, not the seat order"
        );
    }

    #[test]
    fn no_sequence_when_the_display_joins_a_fight_already_in_progress() {
        use crate::battle::{CombatFeed, FeedMode};
        for (live, outcomes, round) in [(false, 0, 1), (true, 2, 1), (true, 0, 2)] {
            let mut stage = stage(false);
            let mut feed = CombatFeed::new(FeedMode::HoldForCues);
            feed.apply(&mut stage, &feed_snapshot(live, outcomes, round))
                .unwrap();
            assert!(
                !stage.is_busy(),
                "live={live} outcomes={outcomes} round={round}"
            );
            assert!(!stage.is_rolling_initiative());
        }
    }

    #[test]
    fn a_view_that_shows_someone_already_acted_ends_the_sequence_in_progress() {
        use crate::battle::{CombatFeed, FeedMode};
        let mut stage = stage(false);
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(&mut stage, &feed_snapshot(true, 0, 1)).unwrap();
        run(&mut stage, 1.0);
        assert!(stage.is_rolling_initiative());
        feed.apply(&mut stage, &feed_snapshot(true, 1, 1)).unwrap();
        assert!(!stage.is_rolling_initiative());
        assert!(stage.hud().roll_for_initiative.badges.is_empty());
    }

    #[test]
    fn the_archer_is_mirrored_when_her_target_is_on_the_left_of_the_screen() {
        let mut stage = stage(false);
        let mut archer = token("hero", Side::Hero, Cell::new(3, 4), 12);
        archer.attack = crate::battle::model::AttackStyle::Ranged;
        stage.set_tokens(vec![
            archer.clone(),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(6, 1), 5),
        ]);
        run(&mut stage, 0.2);
        assert!(
            !figure(&stage, "hero").mirror,
            "target to the right: her art as drawn"
        );
        stage.set_tokens(vec![
            archer,
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(0, 1), 5),
        ]);
        run(&mut stage, 0.2);
        assert!(
            figure(&stage, "hero").mirror,
            "target to the left: mirrored"
        );
        assert!(!figure(&stage, "ally").mirror, "only archers mirror");
    }

    // ---- PLAY-H01 attempt 2: health on the battlefield

    const TEST_SHAKE: &[health::Frame] = &[(0.0, 0.0, 0.0), (0.5, 1.0, 0.0), (1.0, 0.0, 0.0)];

    /// Hooks whose reaction encodes what it was asked in its amplitude: the share of the maximum
    /// lost, plus 1 for a critical hit and 10 for a drop to 0.
    fn install_test_hooks() {
        install_hooks(true);
    }

    fn install_hooks(reactive: bool) {
        health::install(health::Hooks {
            tone: |current, maximum, down| {
                Some(if down || current == 0 {
                    Tone::Down
                } else if current * 4 <= maximum {
                    Tone::Critical
                } else {
                    Tone::Healthy
                })
            },
            reaction: if reactive {
                |lost, maximum, critical, dropped| {
                    Some(health::Reaction {
                        frames: TEST_SHAKE,
                        amplitude_m: lost as f32 / maximum as f32
                            + if critical { 1.0 } else { 0.0 }
                            + if dropped { 10.0 } else { 0.0 },
                        duration_s: 0.4,
                        looping: false,
                    })
                }
            } else {
                |_, _, _, _| None
            },
            tremor: health::Reaction {
                frames: TEST_SHAKE,
                amplitude_m: 0.01,
                duration_s: 1.3,
                looping: true,
            },
        });
    }

    /// The furthest the figure strays from where it stands over `seconds`.
    fn strongest_shift(stage: &mut Stage, id: &str, seconds: f32) -> f32 {
        let rest = figure(stage, id).position;
        let mut furthest: f32 = 0.0;
        for _ in 0..(seconds / 0.05) as u32 {
            stage.update(Duration::from_millis(50));
            furthest = furthest.max((figure(stage, id).position - rest).length());
        }
        furthest
    }

    fn hit(damage: u32, critical: bool, after: u32) -> Beat {
        Beat::Attack {
            attacker: TokenId::new("hero"),
            target: TokenId::new("enemy"),
            result: AttackResult::Hit {
                damage,
                critical,
                downed: after == 0,
            },
            target_hit_points_after: Some(after),
        }
    }

    #[test]
    fn a_blow_shakes_the_struck_figure_by_the_share_of_the_bar_it_took_whichever_path_brings_it() {
        install_test_hooks();
        // The staged path: the blow lands at its impact and a drop to 0 is the largest reaction.
        let mut dropped = stage(false);
        dropped.stage(killing_blow()).unwrap();
        run(&mut dropped, 0.5);
        assert!(strongest_shift(&mut dropped, "enemy", 4.0) > 5.0);
        // An ordinary hit of 3 of 12 adds a little to the stage's own recoil; the same hit
        // critical adds a metre more.
        let shift_of = |blow: Beat, reactive: bool| {
            install_hooks(reactive);
            let mut stage = stage(false);
            stage.stage(blow).unwrap();
            run(&mut stage, 0.5);
            strongest_shift(&mut stage, "enemy", 4.0)
        };
        let recoil = shift_of(hit(3, false, 2), false);
        let small = shift_of(hit(3, false, 2), true);
        let big = shift_of(hit(3, true, 2), true);
        assert!(small > recoil && small < recoil + 0.35, "{recoil} {small}");
        assert!(big > small + 0.7, "{big} against {small}");
        install_hooks(true);
        // The unstaged path: a view that simply reports fewer hit points reacts as well, and a
        // view that reports more (a heal) does not.
        let mut direct = stage(false);
        direct.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 3),
        ]);
        let shift = strongest_shift(&mut direct, "enemy", 1.0);
        assert!(shift > 0.1 && shift < 0.3, "{shift}");
        let mut healed = stage(false);
        healed.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 9),
        ]);
        assert_eq!(strongest_shift(&mut healed, "enemy", 1.0), 0.0);
    }

    #[test]
    fn only_a_critical_figure_trembles() {
        install_test_hooks();
        let mut stage = stage(false);
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 2),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        assert_eq!(figure(&stage, "hero").tone, Some(Tone::Critical));
        assert!(strongest_shift(&mut stage, "hero", 2.0) > 0.002);
        assert_eq!(figure(&stage, "ally").tone, Some(Tone::Healthy));
        assert_eq!(strongest_shift(&mut stage, "ally", 2.0), 0.0);
    }

    #[test]
    fn a_health_variant_of_the_picture_changes_with_the_blow_not_before_it() {
        install_test_hooks();
        let variant = format!("/gameplay/imagery/{}", "b".repeat(64));
        let mut stage = stage(false);
        stage.hold_hit_points([&TokenId::new("enemy")]);
        stage.stage(hit(3, false, 2)).unwrap();
        health::set_variants([("ENEMY".to_owned(), variant.clone())]);
        run(&mut stage, 0.3);
        assert_eq!(
            figure(&stage, "enemy").variant,
            None,
            "the blow has not landed"
        );
        run(&mut stage, 3.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 2);
        assert_eq!(figure(&stage, "enemy").variant, Some(variant));
        // Withdrawn variants (a heal above the line) go back to the authored art.
        health::set_variants([]);
        run(&mut stage, 0.2);
        assert_eq!(figure(&stage, "enemy").variant, None);
    }

    #[test]
    fn a_figure_unconscious_at_one_hit_point_is_down_and_not_a_dark_heap() {
        install_test_hooks();
        let mut stage = stage(false);
        let mut knocked_out = token("hero", Side::Hero, Cell::new(3, 4), 1);
        knocked_out.statuses = vec![Status::Unconscious];
        stage.set_tokens(vec![
            knocked_out,
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        let hero = figure(&stage, "hero");
        assert_eq!(
            hero.tone,
            Some(Tone::Down),
            "the knockout flag, not only 0 hit points"
        );
        assert_eq!(hero.slump, 1.0);
        assert!(!hero.downed, "still has a hit point");
        assert!(
            hero.dim <= LAID_LOW_DIM + 1e-6 && hero.dim > 0.0,
            "{}",
            hero.dim
        );
        health::set_variants([(
            "HERO".to_owned(),
            format!("/gameplay/imagery/{}", "c".repeat(64)),
        )]);
        run(&mut stage, 0.2);
        assert_eq!(figure(&stage, "hero").dim, LAID_LOW_DIM_WITH_VARIANT);
        health::set_variants([]);
    }
}
