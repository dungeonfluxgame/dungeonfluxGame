//! The running stage: tokens, the camera rig, playback of director timelines, floating numbers
//! and effects, advanced by explicit elapsed time so every behaviour is testable with a
//! controlled clock. The renderer asks [`Stage::frame`] for a plain description of what to draw.
//! Nothing here decides an outcome; beats arrive already resolved by the server.

use std::{collections::VecDeque, time::Duration};

use super::{
    camera::{CameraPose, CameraRig, ShotName},
    director::{self, Anim, FALL_LANDS, FlashTone, PlanError, ReadyStyle, Step, Timeline},
    feel::{self, FeelState, FeelView, ImpactClass, ImpactEffect},
    float_text,
    framing::{self, Body},
    grid::{Cell, Grid},
    health::{self, Reaction, Tone},
    layout::{Cover, Layout},
    math::{Vec3, ease_in_out},
    model::{
        AttackResult, AttackStyle, BadgeView, Beat, CameraCommand, CameraFocus, DieView,
        FinishingBlow, FloatingText, Highlight, HitPoints, HudState, InitiativeShow, Side, Token,
        TokenId,
    },
    particles::Particles,
    rest_shot,
    scene_file::BattleScene,
    sound::{SoundEvent, SoundKind, voice_for},
    speech,
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
/// The longest a roll card waits for the camera to settle on the fighters.
const ROLL_CARD_MAX_WAIT_S: f32 = 1.6;
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
/// The celebration (or the defeat) after the blow that decides a fight, in seconds from that blow
/// landing: the last foe falls, the finishing card, then a held wide shot of the party with the
/// sting and the heroes reacting. The stage is busy for all of it, so nothing that waits on the
/// stage (the victory card, leaving the battlefield) starts before it is over.
pub const CELEBRATION_SECONDS: f32 = 6.0;
/// The wide shot of the party is held at least this long, however long the blow itself took.
const PARTY_HOLD_MIN_S: f32 = 1.6;
/// The sting sounds this long after the blow lands.
const STING_AT_S: f32 = 0.9;
/// The camera's pan to the party, in seconds.
const PARTY_PAN_S: f32 = 1.1;
/// How much closer than the fitted wide shot the party is held (all stay inside the safe area).
const PARTY_DOLLY: f32 = 0.85;
/// Gap between one hero's reaction and the next, in seconds.
const REACT_STAGGER_S: f32 = 0.3;

/// How far the resting shot leans toward the figure whose turn begins (a share of its distance).
pub const LEAN_DOLLY: f32 = 0.72;
/// The resting shot is refitted (and the camera eased there) only when the new one differs this much.
const REFIT_TARGET_M: f32 = 0.6;
const REFIT_DISTANCE_SHARE: f32 = 0.06;
const REFIT_YAW_DEG: f32 = 4.0;
const REFIT_PAN_S: f32 = 1.0;

/// Attack names waiting for their blows, at most.
const MAX_PENDING_LABELS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ending {
    Victory,
    Defeat,
}

/// The held beat after a deciding blow.
struct Celebration {
    ending: Ending,
    /// Seconds since the blow landed.
    since_s: f32,
    /// Seconds still to hold.
    left_s: f32,
    sting_played: bool,
    /// Seconds since the camera went to the party, once it has.
    party_for_s: Option<f32>,
    /// How many figures have reacted so far.
    reacted: usize,
}

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

/// A blow landing, as the HUD work can subscribe to it: who was hit and how heavily. Presentation
/// only: the outcome is the server's.
#[derive(Clone, Debug, PartialEq)]
pub struct ImpactEvent {
    pub target: TokenId,
    pub attacker: TokenId,
    /// 0 (a graze) to 1 (a knockout): the same number that scales the camera shake.
    pub intensity: f32,
    pub class: ImpactClass,
    pub critical: bool,
    pub knockout: bool,
    /// The target is a hero (the acting side's vignette pulses).
    pub on_hero: bool,
}

/// Camera distance above which a shot counts as a wide shot (it gets the slow orbit drift).
const WIDE_DISTANCE_M: f32 = 9.0;

/// The line being spoken now, for the bubble over the speaker.
pub struct SpeechView<'a> {
    pub token: &'a TokenId,
    pub name: &'a str,
    pub text: &'a str,
    pub line_id: u64,
    pub age_s: f32,
}

/// A spoken line on the stage: who says it and how the camera was before it came in.
struct Speech {
    token: TokenId,
    name: String,
    text: String,
    line_id: u64,
    age_s: f32,
    /// Where the camera was when the line began, to ease back to; `None` when the camera was
    /// busy with a beat and was left alone.
    return_pose: Option<CameraPose>,
}

pub struct FrameView<'a> {
    pub camera: CameraPose,
    pub shake_px: (f32, f32),
    /// Picture roll in degrees (shake and handheld sway).
    pub roll_deg: f32,
    /// Motion blur, radial punch, chromatic split, focus pull and vignette pulse for the post pass.
    pub feel: FeelView,
    /// Particle instances for the renderer (see `particles`).
    pub particles: Vec<f32>,
    /// Far to near, so alpha blending is correct.
    pub figures: Vec<FigureView>,
    pub highlight: Option<&'a Highlight>,
    pub tactical_highlights: &'a [Highlight],
    pub speech: Option<SpeechView<'a>>,
    pub projectiles: Vec<ProjectileView>,
    /// The layout in play and the grid with its obstacles applied.
    pub layout: Option<&'a Layout>,
    pub grid: &'a Grid,
    pub flash: Option<FlashView>,
    pub floats: &'a [FloatingText],
    pub hud: &'a HudState,
    pub clock_s: f32,
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
    tactical_highlights: Vec<Highlight>,
    speech: Option<Speech>,
    celebration: Option<Celebration>,
    /// The attack names waiting for their blow's result: who struck, and with what.
    attack_labels: Vec<(TokenId, String, String)>,
    hud: HudState,
    /// The server's active combatant, which the label and marker show once no beat is playing.
    server_active: Option<TokenId>,
    /// While set, the label and marker follow the beats being played, not the server's active
    /// combatant: its enemies' turns are still waiting to be shown.
    active_held: bool,
    floats: Vec<FloatingText>,
    flash: Option<Flash>,
    clock_s: f32,
    dropped_beats: u32,
    next_float_id: u64,
    roll_card_left_s: f32,
    /// A roll card waiting for the camera to arrive, and how long it has waited.
    pending_roll_card: Option<(super::model::RollCard, f32)>,
    feel: FeelState,
    particles: Particles,
    impacts: VecDeque<ImpactEvent>,
    /// The tempo cue's intensity (0 calm to 1000 peak) that sets the handheld sway.
    tempo_permille: u16,
    /// Battle feel (shake, hit-stop, sway, blur, particles) on or off; off for measurements.
    feel_enabled: bool,
    last_angles: Option<(f32, f32)>,
    /// Who the camera was last framed on, to pull focus when the subject changes.
    last_framing: Option<(TokenId, Option<TokenId>)>,
    /// The resting wide shot the camera was last sent to, while it is the shot being held.
    rest_pose: Option<CameraPose>,
    /// Something the resting shot depends on changed (figures, cells, a beat ended): refit it once
    /// the stage is idle.
    rest_dirty: bool,
    was_idle: bool,
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
    pub fn new(scene: BattleScene) -> Self {
        let opening = scene
            .cameras
            .get(&ShotName::Tactical)
            .copied()
            .expect("scene files are validated to hold every named shot");
        Self {
            rig: CameraRig::new(opening),
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
            tactical_highlights: Vec::new(),
            speech: None,
            celebration: None,
            attack_labels: Vec::new(),
            server_active: None,
            active_held: false,
            hud: HudState::default(),
            floats: Vec::new(),
            flash: None,
            clock_s: 0.0,
            dropped_beats: 0,
            next_float_id: 0,
            roll_card_left_s: 0.0,
            pending_roll_card: None,
            feel: FeelState::new(),
            particles: Particles::default(),
            impacts: VecDeque::new(),
            tempo_permille: 300,
            feel_enabled: true,
            last_angles: None,
            last_framing: None,
            rest_pose: None,
            rest_dirty: false,
            was_idle: true,
        }
    }

    /// Sets the tempo cue's intensity (0 to 1000), which the handheld sway follows: calm is slow
    /// and small, peak quicker and tighter. This is the value the display view already carries.
    pub fn set_tempo_intensity(&mut self, permille: u16) {
        self.tempo_permille = permille.min(1000);
    }

    /// Turns battle feel (shake, hit-stop, slow motion, sway, blur, particles) on or off.
    pub fn set_feel_enabled(&mut self, enabled: bool) {
        self.feel_enabled = enabled;
        if !enabled {
            self.feel = FeelState::new();
        }
    }

    /// Takes the blows that landed since the last call, in order: the single impact event the
    /// HUD work subscribes to (the page forwards it to its sink, as it does the sounds).
    pub fn take_impacts(&mut self) -> Vec<ImpactEvent> {
        self.impacts.drain(..).collect()
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
            voice: runtime
                .token
                .voice
                .unwrap_or_else(|| voice_for(runtime.token.side, runtime.token.figure.as_deref())),
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

    /// Shows the roll card, but only once the camera has settled on the fighters: while it is
    /// still moving to them the card would hang over the wrong picture. It waits at most
    /// [`ROLL_CARD_MAX_WAIT_S`].
    pub fn show_roll_card(&mut self, card: super::model::RollCard) {
        if self.rig.is_panning() || self.camera_move_ahead() {
            self.pending_roll_card = Some((card, 0.0));
        } else {
            self.pending_roll_card = None;
            self.hud.roll_card = Some(card);
            self.roll_card_left_s = ROLL_CARD_SECONDS;
        }
    }

    /// A line is spoken by the figure called `speaker_name`: a bubble opens over it and the camera
    /// eases in on it. Returns false (and shows nothing) when nobody of that name is on the field.
    /// A second line while one is up replaces it. The camera is left alone while a beat plays.
    pub fn speak(&mut self, speaker_name: &str, text: &str, line_id: u64) -> bool {
        let wanted = speaker_name.trim().to_lowercase();
        let Some(runtime) = self
            .tokens
            .iter()
            .find(|runtime| runtime.token.name.trim().to_lowercase() == wanted)
        else {
            return false;
        };
        let token = runtime.token.id.clone();
        let name = runtime.token.name.clone();
        // A replaced line keeps the pose the camera first left.
        let return_pose = match self.speech.take() {
            Some(previous) => previous.return_pose,
            None if self.playing.is_none() && self.queue.is_empty() => Some(self.rig.pose()),
            None => None,
        };
        if return_pose.is_some() {
            let command = CameraCommand {
                shot: ShotName::TurnFocus,
                focus: CameraFocus::Action {
                    actor: token.clone(),
                    target: None,
                    stand: None,
                },
                dolly: speech::CAMERA_DOLLY,
            };
            if let Some((pose, _)) = self.camera_pose_for(&command) {
                self.rig
                    .pan_to(pose, Duration::from_secs_f32(speech::CAMERA_EASE_S));
            }
        }
        self.speech = Some(Speech {
            token,
            name,
            text: text.to_owned(),
            line_id,
            age_s: 0.0,
            return_pose,
        });
        true
    }

    /// The line `line_id` ended (its voice finished): the bubble closes and the camera eases back.
    /// A call for any other line is ignored.
    pub fn end_speech(&mut self, line_id: u64) {
        if self
            .speech
            .as_ref()
            .is_none_or(|speech| speech.line_id != line_id)
        {
            return;
        }
        self.close_speech();
    }

    fn close_speech(&mut self) {
        let Some(speech) = self.speech.take() else {
            return;
        };
        if let Some(pose) = speech.return_pose
            && self.playing.is_none()
            && self.queue.is_empty()
        {
            self.rig
                .pan_to(pose, Duration::from_secs_f32(speech::CAMERA_EASE_S));
        }
    }

    /// Names the attack `attacker` is about to make ("Greatsword", "Bite", "Graze"): the result
    /// shown at its target reads "Name - result". Call it before staging the blow; one attacker's
    /// blows take their names in the order they were given. A name a blow never used is dropped
    /// after a number more arrive, so it cannot attach to a much later blow.
    pub fn set_attack_label(&mut self, attacker: &TokenId, label: &str, damage_type: &str) {
        self.attack_labels
            .push((attacker.clone(), label.to_owned(), damage_type.to_owned()));
        if self.attack_labels.len() > MAX_PENDING_LABELS {
            self.attack_labels.remove(0);
        }
    }

    pub fn set_initiative(&mut self, order: Vec<TokenId>) {
        self.hud.initiative = order;
    }

    /// The server's active combatant. The label and marker take it at once when nothing is
    /// being shown, and otherwise when the beats being shown are over (until then they name the
    /// combatant whose beat is playing).
    pub fn set_active(&mut self, active: Option<TokenId>) {
        self.server_active = active;
        if !self.is_showing_beats() {
            self.hud.active.clone_from(&self.server_active);
        }
    }

    /// A question the server is waiting on, shown where the turn label is; `None` clears it.
    pub fn set_prompt(&mut self, prompt: Option<String>) {
        self.hud.prompt = prompt;
    }

    /// Keeps the label and marker on the combatant whose beat is playing, even between beats,
    /// while later beats of the same server view are still to come.
    pub fn hold_active(&mut self, held: bool) {
        self.active_held = held;
    }

    fn is_showing_beats(&self) -> bool {
        self.active_held || self.playing.is_some() || !self.queue.is_empty()
    }

    /// Stages the server's move of `actor` to `to`: its cell is the new one from now on, and it
    /// walks there when the beat plays. Unlike a token update, nothing else about the figure
    /// changes, so it can be placed in the middle of a view's beats.
    pub fn stage_move(&mut self, actor: &TokenId, to: Cell) -> Result<(), StageError> {
        let Some(index) = self.token_index(actor) else {
            return Err(StageError::UnknownToken(actor.clone()));
        };
        let from = self.tokens[index].token.cell;
        self.tokens[index].token.cell = to;
        if from == to {
            return Ok(());
        }
        self.stage(Beat::Move {
            actor: actor.clone(),
            from,
            to,
        })
    }

    /// How `actor` attacks in the blow about to be staged, from the attack the server resolved.
    pub fn set_attack_style(&mut self, actor: &TokenId, style: AttackStyle) {
        if let Some(index) = self.token_index(actor) {
            self.tokens[index].token.attack = style;
        }
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
        let command = CameraCommand {
            shot: ShotName::CombatEst,
            focus: CameraFocus::Wide,
            dolly: 1.0,
        };
        if let Some((pose, _)) = self.camera_pose_for(&command) {
            self.rest_pose = Some(pose);
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

    /// Beats that could not be planned when their turn came (a token vanished meanwhile).
    pub fn dropped_beats(&self) -> u32 {
        self.dropped_beats
    }

    pub fn is_busy(&self) -> bool {
        self.playing.is_some()
            || !self.queue.is_empty()
            || self.feel.is_active()
            || self.celebration.is_some()
    }

    pub fn tokens(&self) -> impl Iterator<Item = &Token> {
        self.tokens.iter().map(|runtime| &runtime.token)
    }

    pub fn camera_pose(&self) -> CameraPose {
        self.rig.pose()
    }

    /// Which shot the camera is holding, for a harness: `wide` is the solved resting shot, `action`
    /// anything else (a lean at the start of a turn, a strike, a speaker, a finishing blow).
    pub fn camera_label(&self) -> &'static str {
        if self.rest_pose.is_some() {
            "wide"
        } else {
            "action"
        }
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
        if let Some(rest) = self.wide_pose() {
            self.rest_pose = Some(rest);
            self.rig.pan_to(rest, Duration::from_millis(2600));
        } else if let Some(combat) = self.scene.cameras.get(&ShotName::CombatEst) {
            self.rig.pan_to(*combat, Duration::from_millis(2600));
        }
    }

    /// Replaces the set of combatants. While a beat is playing, the new set is held and applied
    /// when the stage goes idle so a late view cannot teleport a figure mid-step.
    /// Render the server's legal movement and target plan; this makes no legality decision.
    pub fn set_tactical_highlights(&mut self, reachable: Vec<Cell>, targets: Vec<Cell>) {
        self.tactical_highlights = vec![
            Highlight {
                kind: super::model::HighlightKind::Attack,
                cells: targets,
            },
            Highlight {
                kind: super::model::HighlightKind::Move,
                cells: reachable,
            },
        ];
        self.rest_dirty = true;
    }
    /// The highlights to draw: on an enemy's turn the reachable cells (drawn in hero blue) are
    /// left out; the attack highlight, which comes first, stays.
    fn shown_tactical_highlights(&self) -> &[Highlight] {
        let enemy_turn = self.hud.active.as_ref().is_some_and(|active| {
            self.tokens
                .iter()
                .any(|runtime| &runtime.token.id == active && runtime.token.side == Side::Enemy)
        });
        if enemy_turn {
            let attack_only = self.tactical_highlights.len().min(1);
            &self.tactical_highlights[..attack_only]
        } else {
            &self.tactical_highlights
        }
    }

    pub fn set_tokens(&mut self, tokens: Vec<Token>) {
        if self.is_busy() {
            self.pending_tokens = Some(tokens);
            return;
        }
        self.pending_tokens = None;
        self.rest_dirty = true;
        self.tokens
            .retain(|runtime| tokens.iter().any(|token| token.id == runtime.token.id));
        // Figures the server moved since the last view: they walk there, they are not teleported.
        let mut moves: Vec<(TokenId, Cell, Cell)> = Vec::new();
        let had_figures = !self.tokens.is_empty();
        for incoming in tokens {
            let centre = self.grid.cell_centre(incoming.cell);
            match self
                .tokens
                .iter_mut()
                .find(|runtime| runtime.token.id == incoming.id)
            {
                Some(runtime) => {
                    if runtime.token.cell != incoming.cell {
                        if had_figures {
                            moves.push((incoming.id.clone(), runtime.token.cell, incoming.cell));
                        } else {
                            runtime.position = centre;
                        }
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
        for (actor, from, to) in moves {
            let _ = self.stage(Beat::Move { actor, from, to });
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
            Beat::Move { actor, .. } => (!known(actor)).then(|| actor.clone()),
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
            match director::plan(&self.grid, &self.resting_tokens(), &beat) {
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
        // Real time drives the camera, the shake and the particles; the stage's own clock (the
        // timeline, the figures' animations) runs slower or stops during hit-stop and slow motion.
        let real_dt = elapsed.as_secs_f32().min(0.25);
        self.clock_s += real_dt;
        if self.feel_enabled {
            self.feel.advance(real_dt);
            self.particles.advance(real_dt);
        }
        let scale = if self.feel_enabled {
            self.feel.time_scale()
        } else {
            1.0
        };
        let sim = elapsed.mul_f32(scale);
        let dt = sim.as_secs_f32().min(0.25);
        self.rig.update(elapsed);
        self.observe_camera_speed(real_dt);
        if let Some(speech) = &mut self.speech {
            speech.age_s += real_dt;
            if speech.age_s >= speech::MAX_LINE_S {
                self.close_speech();
            }
        }
        if self.playing.is_none() {
            self.start_next();
        }
        self.advance_playback(sim);
        if !self.is_showing_beats() && self.hud.active != self.server_active {
            self.hud.active.clone_from(&self.server_active);
        }
        self.advance_celebration(real_dt);
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
                if self.feel_enabled {
                    self.particles.dust(runtime.position);
                }
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
        if let Some((card, waited)) = self.pending_roll_card.take() {
            if (self.rig.is_panning() || self.camera_move_ahead()) && waited < ROLL_CARD_MAX_WAIT_S
            {
                self.pending_roll_card = Some((card, waited + real_dt));
            } else {
                self.hud.roll_card = Some(card);
                self.roll_card_left_s = ROLL_CARD_SECONDS;
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
        self.floats
            .retain(|text| text.age_s < float_text::lifetime(text.hold_s));
        if let Some(flash) = &mut self.flash {
            flash.age_s += real_dt;
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
        let idle = !self.is_busy();
        if idle && !self.was_idle {
            self.rest_dirty = true;
        }
        self.was_idle = idle;
        if idle && self.rest_dirty {
            self.refit_rest();
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
            CameraFocus::Scene | CameraFocus::Wide => None,
            CameraFocus::Token(id) | CameraFocus::Lean(id) => chest(id),
            CameraFocus::Between(a, b) => Some(chest(a)?.lerp(chest(b)?, 0.5)),
            CameraFocus::Action { actor, target, .. } => match target {
                Some(target) => Some(chest(actor)?.lerp(chest(target)?, 0.55)),
                None => chest(actor),
            },
        }
    }

    /// Whether the camera is about to move: a beat is queued, or the one playing has a camera
    /// step still to come within the next moments.
    fn camera_move_ahead(&self) -> bool {
        match &self.playing {
            Some(playing) => playing.timeline.steps[playing.next..].iter().any(|timed| {
                matches!(timed.step, Step::Camera { .. })
                    && timed.at <= playing.elapsed + Duration::from_millis(1200)
            }),
            None => !self.queue.is_empty(),
        }
    }

    fn body_of(&self, id: &TokenId) -> Option<Body> {
        self.token_index(id).map(|index| {
            let runtime = &self.tokens[index];
            Body {
                feet: runtime.position,
                height_m: runtime.token.clamped_height(),
            }
        })
    }

    /// Feeds the camera's turning speed to the blur: how fast the heading and pitch are changing.
    fn observe_camera_speed(&mut self, real_dt: f32) {
        let pose = self.rig.pose();
        if let Some((yaw, pitch)) = self.last_angles
            && real_dt > 1e-4
        {
            self.feel.observe_camera_speed(
                framing::wrap_deg(pose.yaw_deg - yaw) / real_dt,
                (pose.pitch_deg - pitch) / real_dt,
                real_dt,
            );
        }
        self.last_angles = Some((pose.yaw_deg, pose.pitch_deg));
    }

    /// The resting shot: solved from everyone alive, the cells the server says can be reached or
    /// targeted, and the scene's own wide shot as a hint (see `rest_shot`).
    fn wide_pose(&self) -> Option<CameraPose> {
        let hint = *self.scene.cameras.get(&ShotName::Tactical)?;
        let mut bodies: Vec<Body> = self
            .tokens
            .iter()
            .filter(|runtime| !runtime.downed)
            .map(|runtime| Body {
                feet: runtime.position,
                height_m: runtime.token.clamped_height(),
            })
            .collect();
        if bodies.is_empty() {
            bodies = self
                .tokens
                .iter()
                .map(|runtime| Body {
                    feet: runtime.position,
                    height_m: runtime.token.clamped_height(),
                })
                .collect();
        }
        if bodies.is_empty() {
            return Some(hint);
        }
        let mut ground = Vec::new();
        for highlight in &self.tactical_highlights {
            for cell in &highlight.cells {
                ground.extend(rest_shot::cell_corners(
                    self.grid.cell_centre(*cell),
                    self.grid.cell_m(),
                ));
            }
        }
        // Where the figures stand, a cell each: a fight with nothing to reach still shows its place.
        for body in &bodies {
            ground.extend(rest_shot::cell_corners(body.feet, self.grid.cell_m()));
        }
        Some(rest_shot::rest_pose(&rest_shot::RestInput {
            bodies: &bodies,
            ground: &ground,
            hint: &hint,
            previous_yaw: Some(self.rig.pose().yaw_deg),
            aspect: framing::FRAMED_ASPECT,
        }))
    }

    /// Eases the camera to a refitted resting shot when what it depends on changed and it is the
    /// shot being held, if the new one is noticeably different.
    fn refit_rest(&mut self) {
        self.rest_dirty = false;
        let Some(held) = self.rest_pose else {
            return;
        };
        if self.speech.is_some() {
            self.rest_dirty = true;
            return;
        }
        let Some(fresh) = self.wide_pose() else {
            return;
        };
        let moved = (fresh.target - held.target).length();
        let changed = moved > REFIT_TARGET_M
            || (fresh.distance / held.distance - 1.0).abs() > REFIT_DISTANCE_SHARE
            || framing::wrap_deg(fresh.yaw_deg - held.yaw_deg).abs() > REFIT_YAW_DEG;
        if changed {
            self.rest_pose = Some(fresh);
            self.rig.pan_to(fresh, Duration::from_secs_f32(REFIT_PAN_S));
        }
    }

    /// The pose for a camera command, and whether reaching it is a cut (a swing too big to pan).
    fn camera_pose_for(&self, command: &CameraCommand) -> Option<(CameraPose, bool)> {
        match &command.focus {
            CameraFocus::Wide => return Some((self.wide_pose()?.dollied(command.dolly), false)),
            CameraFocus::Lean(id) => {
                let chest = self.focus_point(&CameraFocus::Token(id.clone()))?;
                return Some((
                    self.wide_pose()?
                        .aimed_at(chest)
                        .dollied(LEAN_DOLLY * command.dolly),
                    false,
                ));
            }
            _ => {}
        }
        let base = *self.scene.cameras.get(&command.shot)?;
        if let CameraFocus::Action {
            actor,
            target,
            stand,
        } = &command.focus
        {
            let actor_body = self.body_of(actor)?;
            let target_body = match target {
                Some(id) => Some(self.body_of(id)?),
                None => None,
            };
            // Everywhere either fighter stands during the beat, so one pose holds them all.
            let mut also = Vec::new();
            if let Some(cell) = stand {
                also.push(Body {
                    feet: self.grid.cell_centre(*cell),
                    ..actor_body
                });
            }
            if let Some(target_body) = &target_body {
                let line = flat_direction(actor_body.feet, target_body.feet);
                also.push(Body {
                    feet: target_body.feet + line * 0.5,
                    ..*target_body
                });
            }
            let bystanders: Vec<Body> = self
                .tokens
                .iter()
                .filter(|runtime| {
                    &runtime.token.id != actor && Some(&runtime.token.id) != target.as_ref()
                })
                .map(|runtime| Body {
                    feet: runtime.position,
                    height_m: runtime.token.clamped_height(),
                })
                .collect();
            let home_yaw = self
                .scene
                .cameras
                .get(&ShotName::Tactical)
                .map(|pose| pose.yaw_deg);
            let shot = framing::action_shot_from(
                &base,
                &self.rig.pose(),
                &actor_body,
                target_body.as_ref(),
                &also,
                &bystanders,
                framing::FRAMED_ASPECT,
                home_yaw,
            );
            return Some((shot.pose.dollied(command.dolly), shot.cut));
        }
        let aimed = match self.focus_point(&command.focus) {
            Some(point) => base.aimed_at(point),
            None => base,
        };
        Some((aimed.dollied(command.dolly), false))
    }

    /// A blow landed: shake, hit-stop, flash, sparks and the impact event, scaled by its size.
    fn land_impact(
        &mut self,
        attacker: &TokenId,
        target: &TokenId,
        class: ImpactClass,
        share: f32,
        ranged: bool,
    ) {
        let weight = feel::weight(class, share);
        let (Some(from), Some(to)) = (self.token_index(attacker), self.token_index(target)) else {
            return;
        };
        let on_hero = self.tokens[to].token.side == Side::Hero;
        let chest = self.tokens[to].position
            + Vec3::new(0.0, self.tokens[to].token.clamped_height() * 0.55, 0.0);
        let away = flat_direction(self.tokens[from].position, self.tokens[to].position);
        if self.feel_enabled {
            self.feel.impact(ImpactEffect {
                class,
                weight,
                on_hero,
            });
            self.particles
                .sparks(chest, away, if ranged { weight * 0.4 } else { weight });
        }
        let strength = feel::flash_strength(class);
        if strength > 0.0 {
            self.flash = Some(Flash {
                tone: if class >= ImpactClass::Critical {
                    FlashTone::Critical
                } else {
                    FlashTone::Impact
                },
                strength,
                age_s: 0.0,
            });
        }
        self.impacts.push_back(ImpactEvent {
            target: target.clone(),
            attacker: attacker.clone(),
            intensity: weight,
            class,
            critical: class == ImpactClass::Critical,
            knockout: class == ImpactClass::Knockout,
            on_hero,
        });
        // Nobody drains the events in a hidden tab: keep only the newest few.
        while self.impacts.len() > 16 {
            self.impacts.pop_front();
        }
    }

    fn apply(&mut self, step: Step) {
        match step {
            Step::SetActive(active) => self.hud.active = active,
            Step::Camera { command, pan } => {
                if let Some((pose, cut)) = self.camera_pose_for(&command) {
                    self.rest_pose = (command.focus == CameraFocus::Wide).then_some(pose);
                    let framing = match &command.focus {
                        CameraFocus::Action { actor, target, .. } => {
                            Some((actor.clone(), target.clone()))
                        }
                        _ => None,
                    };
                    // A new subject pulls focus, whether the camera pans or cuts to it.
                    let new_subject = framing != self.last_framing;
                    if self.feel_enabled && new_subject {
                        self.feel.focus_pull();
                    }
                    self.last_framing = framing;
                    // A move that would lose the fighters part of the way is a cut, not a pan.
                    let mut cut = cut;
                    if let CameraFocus::Action { actor, target, .. } = &command.focus
                        && !cut
                    {
                        let bodies: Vec<Body> = std::iter::once(actor)
                            .chain(target.as_ref())
                            .filter_map(|id| self.body_of(id))
                            .collect();
                        cut = !framing::pan_keeps_in_view(
                            &self.rig.pose(),
                            &pose,
                            &bodies,
                            framing::FRAMED_ASPECT,
                        );
                    }
                    if cut {
                        self.rig.cut_to(pose);
                    } else {
                        self.rig.pan_to(pose, pan);
                    }
                }
            }
            Step::Impact {
                attacker,
                target,
                class,
                share,
                ranged,
            } => self.land_impact(&attacker, &target, class, share, ranged),
            Step::Whoosh => {
                if self.feel_enabled {
                    self.feel.miss();
                }
            }
            Step::SlowMotion => {
                if self.feel_enabled {
                    self.feel.slow_motion();
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
                    strength,
                    age_s: 0.0,
                });
            }
            Step::Float {
                token,
                text,
                tone,
                attacker,
                downed,
            } => {
                self.next_float_id += 1;
                // The attack's name, when the view gave one for this attacker's blow.
                let attack = attacker.and_then(|attacker| {
                    let at = self
                        .attack_labels
                        .iter()
                        .position(|(id, ..)| *id == attacker)?;
                    let (_, name, damage_type) = self.attack_labels.remove(at);
                    Some((name, damage_type))
                });
                self.floats.push(FloatingText {
                    id: self.next_float_id,
                    token,
                    text: float_text::compose(
                        attack
                            .as_ref()
                            .map(|(name, kind)| (name.as_str(), kind.as_str())),
                        &text,
                        tone,
                        downed,
                    ),
                    tone,
                    age_s: 0.0,
                    hold_s: float_text::hold_for(tone, downed),
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
                if let Some((pose, _)) = self.camera_pose_for(&command) {
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
                self.hud.roll_for_initiative.dice.push(DieView {
                    token,
                    face: die,
                    modifier,
                    total,
                    age_s: 0.0,
                    tumble_s: DIE_TUMBLE_SECONDS,
                    landed: false,
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
        let grid = &self.grid;
        let target = grid.cell_centre(cell);
        let runtime = &mut self.tokens[index];
        let from = grid.cell_at(runtime.position).unwrap_or(runtime.token.cell);
        let path = grid.path(from, cell).unwrap_or_default();
        if per_step.is_zero() {
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

    /// Whether `runtime` is down (or going down this moment).
    fn is_down(runtime: &Runtime, falling: &TokenId) -> bool {
        runtime.token.id == *falling
            || runtime.downed
            || matches!(&runtime.anim, Some(active) if matches!(active.anim, Anim::Fall))
            || runtime.shown_hp.is_some_and(|hp| hp.current == 0)
    }

    /// A figure starts to fall: if that leaves no one standing on its side, the fight is decided
    /// and the held beat begins from this blow.
    fn note_fall(&mut self, id: &TokenId) {
        if self.celebration.is_some() {
            return;
        }
        let Some(side) = self
            .token_index(id)
            .map(|index| self.tokens[index].token.side)
        else {
            return;
        };
        let everyone_down = self
            .tokens
            .iter()
            .filter(|runtime| runtime.token.side == side)
            .all(|runtime| Self::is_down(runtime, id));
        if !everyone_down {
            return;
        }
        let ending = if side == Side::Enemy {
            Ending::Victory
        } else {
            Ending::Defeat
        };
        self.celebration = Some(Celebration {
            ending,
            since_s: 0.0,
            left_s: CELEBRATION_SECONDS,
            sting_played: false,
            party_for_s: None,
            reacted: 0,
        });
    }

    /// The camera, the sting and the reactions of the held beat; ends it when its time is up.
    fn advance_celebration(&mut self, real_dt: f32) {
        let Some(mut beat) = self.celebration.take() else {
            return;
        };
        beat.since_s += real_dt;
        beat.left_s -= real_dt;
        let ending = beat.ending;
        if !beat.sting_played && beat.since_s >= STING_AT_S {
            beat.sting_played = true;
            let kind = match ending {
                Ending::Victory => SoundKind::VictorySting,
                Ending::Defeat => SoundKind::DefeatSting,
            };
            let heard_by = self
                .tokens
                .iter()
                .find(|runtime| {
                    runtime.token.side
                        == if ending == Ending::Victory {
                            Side::Hero
                        } else {
                            Side::Enemy
                        }
                })
                .map(|runtime| runtime.token.id.clone());
            if let Some(token) = heard_by {
                self.emit(kind, &token);
            }
        }
        // The blow's own timeline (the fall and the finishing card) plays out first; then the
        // camera goes to the party and holds there.
        let timeline_over = self.playing.is_none() && self.queue.is_empty();
        if timeline_over && beat.party_for_s.is_none() {
            beat.party_for_s = Some(0.0);
            beat.left_s = beat.left_s.max(PARTY_HOLD_MIN_S);
            if let Some(pose) = self.party_pose(ending) {
                self.rig.pan_to(pose, Duration::from_secs_f32(PARTY_PAN_S));
            }
        } else if let Some(held) = &mut beat.party_for_s {
            *held += real_dt;
        }
        if let Some(held) = beat.party_for_s {
            // The figures react one after another: the heroes raise their weapons in a victory,
            // the enemies snarl over a defeat.
            let side = if ending == Ending::Victory {
                Side::Hero
            } else {
                Side::Enemy
            };
            let actors: Vec<TokenId> = self
                .tokens
                .iter()
                .filter(|runtime| runtime.token.side == side && !runtime.downed)
                .map(|runtime| runtime.token.id.clone())
                .collect();
            while beat.reacted < actors.len()
                && held >= beat.reacted as f32 * REACT_STAGGER_S + 0.25
            {
                let style = if ending == Ending::Victory {
                    ReadyStyle::Brandish
                } else {
                    ReadyStyle::Snarl
                };
                self.start_anim(&actors[beat.reacted], Anim::Ready { style });
                beat.reacted += 1;
            }
        }
        if beat.left_s > 0.0 {
            self.celebration = Some(beat);
        }
    }

    /// The wide shot of the party on the field: every hero in frame (the fallen ones too, in a
    /// defeat), from the side the camera already holds.
    fn party_pose(&self, ending: Ending) -> Option<CameraPose> {
        let base = *self.scene.cameras.get(&ShotName::TurnFocus)?;
        let party: Vec<Body> = self
            .tokens
            .iter()
            .filter(|runtime| runtime.token.side == Side::Hero)
            .map(|runtime| Body {
                feet: runtime.position,
                height_m: runtime.token.clamped_height(),
            })
            .collect();
        let (first, rest) = party.split_first()?;
        let home_yaw = self
            .scene
            .cameras
            .get(&ShotName::Tactical)
            .map(|pose| pose.yaw_deg);
        let _ = ending;
        let shot = framing::action_shot_from(
            &base,
            &self.rig.pose(),
            first,
            None,
            rest,
            &[],
            framing::FRAMED_ASPECT,
            home_yaw,
        );
        Some(shot.pose.dollied(PARTY_DOLLY))
    }

    fn start_anim(&mut self, id: &TokenId, anim: Anim) {
        let Some(index) = self.token_index(id) else {
            return;
        };
        if matches!(anim, Anim::Fall) {
            self.note_fall(id);
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
        let view = if self.feel_enabled {
            self.feel.view()
        } else {
            FeelView::default()
        };
        let mut camera = self.rig.pose();
        let mut roll_deg = view.shake_roll_deg;
        if self.feel_enabled {
            // The living camera: a slow handheld sway whenever it is otherwise still (less while
            // it moves), plus the slow orbit drift on a wide shot.
            let sway = feel::sway(
                self.clock_s,
                self.tempo_permille,
                camera.distance > WIDE_DISTANCE_M,
            );
            let calm = if self.rig.is_panning() { 0.4 } else { 1.0 };
            camera.yaw_deg += sway.yaw_deg * calm;
            camera.pitch_deg = (camera.pitch_deg + sway.pitch_deg * calm).clamp(2.0, 89.0);
            roll_deg += sway.roll_deg * calm;
        }
        let rig_shake = self.rig.shake_offset_px();
        FrameView {
            camera,
            roll_deg,
            feel: view,
            particles: if self.feel_enabled {
                self.particles.instances()
            } else {
                Vec::new()
            },
            shake_px: (rig_shake.0 + view.shake_px.0, rig_shake.1 + view.shake_px.1),
            figures,
            highlight: self.highlight.as_ref(),
            tactical_highlights: self.shown_tactical_highlights(),
            speech: self.speech.as_ref().map(|speech| SpeechView {
                token: &speech.token,
                name: &speech.name,
                text: &speech.text,
                line_id: speech.line_id,
                age_s: speech.age_s,
            }),
            projectiles: self.projectiles.iter().map(projectile_view).collect(),
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
        let pose = status::pose_for(statuses);
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
        if resting && pose.idle > 0.0 {
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
        let position = self.shaken(runtime, tone, position, rest);
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
    ///
    /// It composes with the stage's own movement of the figure (the recoil from the blow, a walk)
    /// and never works against it: the sideways shake is taken perpendicular to however far the
    /// figure is already displaced from its cell, so the total movement is at every moment at
    /// least what the recoil alone moves. The reaction's clock is the stage's simulation clock,
    /// so a hit-stop holds it where it began at contact instead of skipping it.
    fn shaken(&self, runtime: &Runtime, tone: Option<Tone>, position: Vec3, rest: Vec3) -> Vec3 {
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
        let mut axis = Vec3::new(right.x, 0.0, right.z).normalized();
        let displaced = Vec3::new(position.x - rest.x, 0.0, position.z - rest.z);
        if displaced.length() > 1e-3 {
            let along = displaced.normalized();
            let mut across = axis - along * axis.dot(along);
            if across.length() < 1e-3 {
                across = Vec3::new(-along.z, 0.0, along.x);
            }
            axis = across.normalized();
        }
        let mut moved = position + axis * side;
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

fn projectile_view(projectile: &Projectile) -> ProjectileView {
    let t = (projectile.age_s / projectile.flight_s).clamp(0.0, 1.0);
    let head = arrow_at(projectile, t);
    let tangent = (head - arrow_at(projectile, (t - 0.03).max(0.0))).normalized();
    let tangent = if tangent == Vec3::ZERO {
        (projectile.to - projectile.from).normalized()
    } else {
        tangent
    };
    let tail = head - tangent * 0.85;
    let trail = arrow_at(projectile, (t - 0.28).max(0.0));
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
        model::{AttackResult, HitPoints},
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
            voice: None,
        }
    }

    fn stage() -> Stage {
        let scene = load_embedded("salt-quay").unwrap().unwrap();
        let mut stage = Stage::new(scene);
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
        let stage = stage();
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
        let mut stage = Stage::new(scene);
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
        let stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
    fn the_hit_point_bar_changes_only_at_the_moment_of_impact() {
        let mut stage = stage();
        stage.stage(killing_blow()).unwrap();
        run(&mut stage, 1.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 5);
        run(&mut stage, 3.0);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 0);
    }

    #[test]
    fn an_ordinary_hit_returns_the_attacker_to_formation() {
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
    fn the_finishing_shot_has_the_victory_lens_then_the_camera_holds_the_party_wide() {
        let mut stage = stage();
        stage.stage(killing_blow()).unwrap();
        // While the finishing card is up: the victory lens and pitch, the fallen foe and the
        // attacker both in frame.
        let mut on_victory_shot = false;
        let mut ended = false;
        for _ in 0..1200 {
            stage.update(Duration::from_millis(20));
            let victory = stage.scene().cameras[&ShotName::Victory];
            let pose = stage.camera_pose();
            if stage.hud().finishing_blow.is_some()
                && !stage.rig.is_panning()
                && (pose.pitch_deg - victory.pitch_deg).abs() < 0.01
                && (pose.fov_deg - victory.fov_deg).abs() < 0.01
                && projected_inside_safe(&stage, "hero")
                && projected_inside_safe(&stage, "enemy")
            {
                on_victory_shot = true;
            }
            if !stage.is_busy() {
                ended = true;
                break;
            }
        }
        assert!(on_victory_shot && ended);
        // The beat ends on the party held wide: every hero in frame.
        assert!(projected_inside_safe(&stage, "hero"));
        assert!(projected_inside_safe(&stage, "ally"));
    }

    /// The deciding blow is followed by a held beat of CELEBRATION_SECONDS from the blow
    /// landing, the stage is busy for all of it, and the victory sting and the heroes' reactions
    /// happen inside it.
    #[test]
    fn the_last_foe_down_holds_a_six_second_victory_beat_with_the_stage_busy_throughout() {
        assert_eq!(CELEBRATION_SECONDS, 6.0);
        let mut stage = stage();
        stage.stage(killing_blow()).unwrap();
        let mut impact_at = None;
        let mut idle_at = None;
        let mut sting_at = None;
        let mut elapsed = 0.0f32;
        for _ in 0..2000 {
            stage.update(Duration::from_millis(20));
            elapsed += 0.02;
            if impact_at.is_none() && stage.take_impacts().into_iter().next().is_some() {
                impact_at = Some(elapsed);
            }
            for event in stage.take_sounds() {
                if event.kind == SoundKind::VictorySting && sting_at.is_none() {
                    sting_at = Some(elapsed);
                }
            }
            if impact_at.is_some() && !stage.is_busy() && idle_at.is_none() {
                idle_at = Some(elapsed);
            }
            if idle_at.is_some() {
                break;
            }
        }
        let (impact, idle) = (
            impact_at.expect("a blow landed"),
            idle_at.expect("the beat ended"),
        );
        assert!(
            idle - impact >= CELEBRATION_SECONDS - 0.5,
            "busy {} s after the blow",
            idle - impact
        );
        assert!(
            idle - impact < CELEBRATION_SECONDS + 2.5,
            "{}",
            idle - impact
        );
        let sting = sting_at.expect("the victory sting played");
        assert!(
            sting > impact && sting < idle,
            "sting {sting} within {impact}..{idle}"
        );
    }

    /// The stage never reports idle between the blow and the end of the beat, so whatever waits
    /// on it (the victory card, leaving the battlefield) cannot start early.
    #[test]
    fn the_stage_is_never_idle_between_the_deciding_blow_and_the_end_of_the_beat() {
        let mut stage = stage();
        stage.stage(killing_blow()).unwrap();
        let mut landed = false;
        let mut gaps = 0;
        let mut frames_after = 0;
        for _ in 0..2000 {
            stage.update(Duration::from_millis(20));
            landed |= stage.take_impacts().into_iter().next().is_some();
            if landed {
                frames_after += 1;
                if !stage.is_busy() {
                    break;
                }
            }
        }
        assert!(landed);
        assert!(frames_after as f32 * 0.02 >= CELEBRATION_SECONDS - 0.5);
        for _ in 0..50 {
            stage.update(Duration::from_millis(20));
            if stage.is_busy() {
                gaps += 1;
            }
        }
        assert_eq!(gaps, 0, "idle stays idle once the beat is over");
    }

    #[test]
    fn a_blow_that_leaves_another_foe_standing_holds_no_celebration() {
        let mut stage = stage();
        let mut tokens: Vec<Token> = stage.tokens().cloned().collect();
        tokens.push(token("enemy2", Side::Enemy, Cell::new(6, 1), 5));
        stage.set_tokens(tokens);
        stage.stage(killing_blow()).unwrap();
        let mut sting = false;
        for _ in 0..1500 {
            stage.update(Duration::from_millis(20));
            sting |= stage
                .take_sounds()
                .iter()
                .any(|event| event.kind == SoundKind::VictorySting);
        }
        assert!(!sting, "one foe is still standing");
        assert!(!stage.is_busy());
    }

    #[test]
    fn the_last_hero_down_holds_a_defeat_beat_of_the_same_length_with_its_own_sting() {
        let mut stage = stage();
        for target in ["hero", "ally"] {
            stage
                .stage(Beat::Attack {
                    attacker: TokenId::new("enemy"),
                    target: TokenId::new(target),
                    result: AttackResult::Hit {
                        damage: 20,
                        critical: false,
                        downed: true,
                    },
                    target_hit_points_after: Some(0),
                })
                .unwrap();
        }
        let mut sting_at = None;
        let mut victory_sting = false;
        let mut elapsed = 0.0f32;
        let mut last_busy = 0.0f32;
        for _ in 0..3000 {
            stage.update(Duration::from_millis(20));
            elapsed += 0.02;
            for event in stage.take_sounds() {
                victory_sting |= event.kind == SoundKind::VictorySting;
                if event.kind == SoundKind::DefeatSting && sting_at.is_none() {
                    sting_at = Some(elapsed);
                }
            }
            if stage.is_busy() {
                last_busy = elapsed;
            }
        }
        let sting = sting_at.expect("the defeat sting played");
        assert!(!victory_sting);
        assert!(
            last_busy - sting >= CELEBRATION_SECONDS - STING_AT_S - 0.6,
            "held {} s after the sting",
            last_busy - sting
        );
    }

    #[test]
    fn projecting_a_cell_centre_lands_inside_the_screen_for_the_establishing_shot() {
        let mut stage = stage();
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
    fn voices_follow_the_token_and_the_surface_the_scene() {
        let mut stage = stage();
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
        let thud = seen
            .iter()
            .find(|event| event.kind == SoundKind::BodyFall)
            .unwrap();
        assert_eq!(thud.surface, stage.scene().surface);
    }

    #[test]
    fn the_sound_queue_is_bounded_and_counts_what_it_dropped() {
        let mut stage = stage();
        for _ in 0..(MAX_PENDING_SOUNDS + 5) {
            stage.emit(SoundKind::HealBreath, &TokenId::new("hero"));
        }
        assert_eq!(stage.take_sounds().len(), MAX_PENDING_SOUNDS);
        assert_eq!(stage.dropped_sounds(), 5);
    }

    fn archer_stage() -> Stage {
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
        run(&mut stage, 1.0);
        let at_one: Vec<f32> = ["hero", "ally", "enemy"]
            .iter()
            .map(|id| figure(&stage, id).lean)
            .collect();
        assert!(at_one[0] != at_one[1] && at_one[1] != at_one[2]);
        run(&mut stage, 3.0);
        assert!(figure(&stage, "hero").lean != at_one[0]);
    }

    #[test]
    fn the_target_ring_shows_during_the_focus_and_clears_at_contact() {
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = Stage::new(scene);
        assert!(stage.set_layout("wall"));
        assert!(!stage.set_layout("missing"));
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 5), 12),
            token("enemy", Side::Enemy, Cell::new(3, 0), 5),
        ]);
        stage
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
        run(&mut stage, 6.0);
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
            voice: None,
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
            voice: None,
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
        let mut stage = stage();
        // The server's active combatant is the first to act.
        stage.set_active(Some(TokenId::new("enemy")));
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
        let mut stage = stage();
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
    fn the_camera_whips_to_each_stop_with_a_zoom_punch_on_arrival() {
        let mut lively = stage();
        lively
            .stage(Beat::Initiative {
                rolls: rolls_for(&["hero", "enemy", "ally"]),
            })
            .unwrap();
        let mut closest = f32::MAX;
        let mut panned = false;
        for _ in 0..900 {
            lively.update(Duration::from_millis(20));
            // A whip travels farther than it needs to and comes back: the lens punches in.
            closest = closest.min(lively.camera_pose().fov_deg);
            panned |= lively.rig.is_panning();
        }
        assert!(panned);
        let victory = lively.scene().cameras[&ShotName::Victory].fov_deg;
        assert!(
            closest < victory * 0.97,
            "zoom punch on arrival: {closest} vs {victory}"
        );
        assert!(!lively.is_busy());
    }

    #[test]
    fn a_newer_view_cuts_the_sequence_short_and_goes_to_the_current_turn() {
        let mut stage = stage();
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
            voice: None,
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
                    weapon: String::new(),
                    grazed: false,
                    damage_type: String::new(),
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
        let mut stage = stage();
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
            let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        run(&mut stage, 6.0);
        assert!(
            figure(&stage, "hero").mirror,
            "target to the left: mirrored"
        );
        assert!(!figure(&stage, "ally").mirror, "only archers mirror");
    }

    /// A fixed step of one thirtieth of a second, as the presented frames at 30 fps advance it.
    fn frame_step(stage: &mut Stage) {
        stage.update(Duration::from_secs_f32(1.0 / 30.0));
    }

    fn body_of_figure(stage: &Stage, id: &str) -> framing::Body {
        let figure = figure(stage, id);
        framing::Body {
            feet: figure.position,
            height_m: figure.height_m,
        }
    }

    fn projected_inside_safe(stage: &Stage, id: &str) -> bool {
        framing::in_safe_area(
            &stage.rig.pose(),
            framing::FRAMED_ASPECT,
            &body_of_figure(stage, id),
        )
    }

    fn ordinary_hit(attacker: &str, target: &str, damage: u32) -> Beat {
        Beat::Attack {
            attacker: TokenId::new(attacker),
            target: TokenId::new(target),
            result: AttackResult::Hit {
                damage,
                critical: false,
                downed: false,
            },
            target_hit_points_after: Some(3),
        }
    }

    /// Plays an attack frame by frame: from the moment the camera has settled until the beat is
    /// over both fighters are inside the safe area, and the lunge and contact play on a camera
    /// that is not moving.
    #[test]
    fn both_fighters_stay_in_frame_through_a_whole_attack_and_the_camera_rests_at_contact() {
        let mut stage = stage();
        for _ in 0..30 {
            frame_step(&mut stage);
        }
        stage.stage(ordinary_hit("hero", "enemy", 2)).unwrap();
        let mut frames = 0;
        let mut poses = Vec::new();
        let mut frozen_at = Vec::new();
        while stage.is_busy() && frames < 900 {
            frame_step(&mut stage);
            frames += 1;
            poses.push(stage.rig.pose());
            if stage.feel.is_frozen() {
                frozen_at.push(frames - 1);
            }
            if stage.rig.is_panning() {
                continue;
            }
            assert!(
                projected_inside_safe(&stage, "hero"),
                "frame {frames}: attacker out of frame"
            );
            assert!(
                projected_inside_safe(&stage, "enemy"),
                "frame {frames}: target out of frame"
            );
        }
        assert!(frames < 900, "the beat ends");
        // The hit-stop is at contact: for a few frames either side of it the camera is at rest.
        let (first, last): (usize, usize) = (frozen_at[0], *frozen_at.last().unwrap());
        let around = &poses[first.saturating_sub(6)..=(last + 6).min(poses.len() - 1)];
        assert!(
            around.windows(2).all(|pair| pair[0] == pair[1]),
            "camera at rest at contact"
        );
    }

    /// The live defect: Brakka attacks the goblin with Sylvie elsewhere on the field. The camera
    /// is never blocked by the bystander.
    #[test]
    fn a_bystander_is_never_in_front_of_the_fighters() {
        let scene = load_embedded("scan-village").unwrap().unwrap();
        let mut stage = Stage::new(scene);
        stage.set_tokens(vec![
            token("brakka", Side::Hero, Cell::new(1, 2), 12),
            token("mudge", Side::Enemy, Cell::new(2, 1), 5),
            token("sylvie", Side::Hero, Cell::new(5, 4), 12),
        ]);
        for _ in 0..20 {
            frame_step(&mut stage);
        }
        stage.stage(ordinary_hit("brakka", "mudge", 2)).unwrap();
        let mut frames = 0;
        while stage.is_busy() && frames < 900 {
            frame_step(&mut stage);
            frames += 1;
            if stage.rig.is_panning() {
                continue;
            }
            let pose = stage.rig.pose();
            let involved = [
                body_of_figure(&stage, "brakka"),
                body_of_figure(&stage, "mudge"),
            ];
            let bystanders = [body_of_figure(&stage, "sylvie")];
            let on_the_pair = involved
                .iter()
                .all(|body| framing::in_safe_area(&pose, framing::FRAMED_ASPECT, body));
            if on_the_pair {
                assert!(
                    !framing::bystander_in_front(
                        &pose,
                        framing::FRAMED_ASPECT,
                        &involved,
                        &bystanders
                    ),
                    "frame {frames}: Sylvie stands in front of the fighters"
                );
            }
        }
    }

    #[test]
    fn a_blow_emits_one_impact_event_scaled_by_its_size_and_a_heavy_one_shakes_harder() {
        let strength = |damage: u32| {
            let mut stage = stage();
            stage.stage(ordinary_hit("hero", "enemy", damage)).unwrap();
            let mut peak = 0.0f32;
            let mut events = Vec::new();
            for _ in 0..600 {
                frame_step(&mut stage);
                let shake = stage.frame().shake_px;
                peak = peak.max(shake.0.abs()).max(shake.1.abs());
                events.extend(stage.take_impacts());
            }
            assert_eq!(events.len(), 1, "one impact event per blow");
            (peak, events[0].intensity, events[0].class)
        };
        let (graze_peak, graze_weight, graze_class) = strength(1);
        let (heavy_peak, heavy_weight, heavy_class) = strength(7);
        assert_eq!(graze_class, ImpactClass::Graze);
        assert_eq!(heavy_class, ImpactClass::Heavy);
        assert!(heavy_weight > graze_weight * 3.0);
        assert!(
            heavy_peak > graze_peak * 2.0,
            "{graze_peak} vs {heavy_peak}"
        );
    }

    #[test]
    fn a_miss_sways_the_camera_without_an_impact_event_or_hit_stop() {
        let mut stage = stage();
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        let mut froze = false;
        for _ in 0..600 {
            frame_step(&mut stage);
            froze |= stage.feel.is_frozen();
        }
        assert!(!froze);
        assert!(stage.take_impacts().is_empty());
    }

    #[test]
    fn hit_stop_freezes_the_figures_for_a_few_frames_and_then_releases() {
        let mut stage = stage();
        stage.stage(ordinary_hit("hero", "enemy", 7)).unwrap();
        let mut frozen_frames = 0;
        let mut previous = None;
        let mut held = true;
        for _ in 0..600 {
            frame_step(&mut stage);
            let enemy = figure(&stage, "enemy");
            let pose = (enemy.position, enemy.red_flash);
            if stage.feel.is_frozen() {
                frozen_frames += 1;
                if let Some(before) = previous {
                    held &= before == pose;
                }
                previous = Some(pose);
            } else {
                previous = None;
            }
        }
        assert!(
            (2..=4).contains(&frozen_frames),
            "a few frames: {frozen_frames}"
        );
        assert!(held, "the figures do not move during hit-stop");
    }

    #[test]
    fn the_finishing_blow_runs_in_slow_motion_and_is_busy_for_all_of_it() {
        let mut stage = stage();
        stage.stage(killing_blow()).unwrap();
        let mut saw_slow = false;
        for _ in 0..900 {
            frame_step(&mut stage);
            saw_slow |= stage.feel.time_scale() < 0.5;
            assert!(
                !stage.feel.is_active() || stage.is_busy(),
                "is_busy covers the slow motion and the hit-stop"
            );
        }
        assert!(saw_slow, "time slows into the finishing blow");
        assert!(!stage.is_busy(), "and it all ends");
    }

    #[test]
    fn the_camera_lives_while_it_waits_and_follows_the_tempo() {
        let sampled = |tempo: u16| {
            let mut stage = stage();
            stage.set_tempo_intensity(tempo);
            let mut yaws = Vec::new();
            for _ in 0..(30 * 30) {
                frame_step(&mut stage);
                yaws.push(stage.frame().camera.yaw_deg);
            }
            yaws
        };
        let calm = sampled(0);
        let peak = sampled(1000);
        let spread = |values: &[f32]| {
            values.iter().cloned().fold(f32::MIN, f32::max)
                - values.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(
            spread(&calm) > 0.2,
            "it drifts when calm: {}",
            spread(&calm)
        );
        // Two frames a second apart differ: the idle camera is alive.
        assert!((calm[60] - calm[90]).abs() > 1e-3);
        assert!(peak != calm);
        // With effects off the camera is dead still, as before.
        let mut off = stage();
        off.set_feel_enabled(false);
        let first = off.frame().camera;
        for _ in 0..120 {
            frame_step(&mut off);
        }
        assert_eq!(off.frame().camera, first);
    }

    /// The riverbed defect: the enemy attacks a hero while a bystander stands elsewhere, and the
    /// camera showed empty ground. At every frame of the enemy's turn, panning or not, the acting
    /// figure's centre is on the screen, and once settled both fighters are inside the safe area.
    #[test]
    fn the_camera_never_shows_a_frame_without_the_acting_figure_on_an_enemy_turn() {
        let scene = load_embedded("scan-riverbed").unwrap().unwrap();
        let mut stage = Stage::new(scene);
        stage.set_tokens(vec![
            token("dagna", Side::Hero, Cell::new(2, 4), 12),
            token("tomas", Side::Enemy, Cell::new(6, 1), 8),
            token("hilde", Side::Hero, Cell::new(4, 4), 12),
        ]);
        for _ in 0..30 {
            frame_step(&mut stage);
        }
        stage.stage(ordinary_hit("tomas", "dagna", 5)).unwrap();
        let mut frames = 0;
        while stage.is_busy() && frames < 900 {
            frame_step(&mut stage);
            frames += 1;
            let pose = stage.rig.pose();
            let attacker = body_of_figure(&stage, "tomas");
            let centre = framing::project_body(&pose, framing::FRAMED_ASPECT, &attacker)
                .map(|(l, t, r, b)| ((l + r) * 0.5, (t + b) * 0.5));
            let Some((x, y)) = centre else {
                panic!("frame {frames}: the attacker is behind the camera");
            };
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "frame {frames}: empty frame, the attacker is at {x:.2},{y:.2}"
            );
        }
    }

    /// A self action (Second Wind) frames the actor alone, clear of the bystander standing on the
    /// camera line, like the entrance roll-call's one clear figure each.
    #[test]
    fn a_self_action_frames_the_actor_clear_of_a_bystander() {
        let scene = load_embedded("scan-village").unwrap().unwrap();
        let mut stage = Stage::new(scene);
        stage.set_tokens(vec![
            token("brakka", Side::Hero, Cell::new(1, 2), 6),
            token("mudge", Side::Enemy, Cell::new(2, 1), 5),
            token("sylvie", Side::Hero, Cell::new(5, 4), 12),
        ]);
        for _ in 0..20 {
            frame_step(&mut stage);
        }
        stage
            .stage(Beat::Heal {
                source: TokenId::new("brakka"),
                target: TokenId::new("brakka"),
                amount: 5,
                target_hit_points_after: Some(11),
            })
            .unwrap();
        let mut settled = 0;
        let mut frames = 0;
        while stage.is_busy() && frames < 600 {
            frame_step(&mut stage);
            frames += 1;
            if stage.rig.is_panning() {
                continue;
            }
            let pose = stage.rig.pose();
            let actor = body_of_figure(&stage, "brakka");
            if !framing::in_safe_area(&pose, framing::FRAMED_ASPECT, &actor) {
                continue;
            }
            settled += 1;
            let others = [
                body_of_figure(&stage, "mudge"),
                body_of_figure(&stage, "sylvie"),
            ];
            assert!(
                !framing::bystander_in_front(&pose, framing::FRAMED_ASPECT, &[actor], &others),
                "frame {frames}: someone stands in front of the actor"
            );
        }
        assert!(
            settled > 10,
            "the actor is framed for most of the beat: {settled}"
        );
    }

    #[test]
    fn a_roll_card_waits_for_the_camera_to_settle_on_the_fighters() {
        let mut stage = stage();
        // Start from the heading the framing will pick, so the move is a pan, not a cut.
        let pick = |stage: &Stage| {
            let attacker = body_of_figure(stage, "hero");
            let target = body_of_figure(stage, "enemy");
            let style = stage.scene().cameras[&ShotName::TurnFocus];
            framing::action_shot_from(
                &style,
                &stage.rig.pose(),
                &attacker,
                Some(&target),
                &[],
                &[],
                framing::FRAMED_ASPECT,
                Some(stage.scene().cameras[&ShotName::Tactical].yaw_deg),
            )
        };
        let first = pick(&stage);
        let near = CameraPose {
            yaw_deg: first.pose.yaw_deg + 30.0,
            ..stage.rig.pose()
        };
        stage.rig.cut_to(near);
        stage.stage(ordinary_hit("hero", "enemy", 2)).unwrap();
        frame_step(&mut stage);
        frame_step(&mut stage);
        // The framing may be reached by a pan or, when a pan would lose the fighters, a cut.
        let panning = stage.rig.is_panning();
        stage.show_roll_card(super::super::model::RollCard {
            value: "14".to_owned(),
            label: "d20".to_owned(),
            explanation: String::new(),
            verdict: "HIT".to_owned(),
            tone: "hit".to_owned(),
        });
        if panning {
            assert!(
                stage.hud().roll_card.is_none(),
                "not while the camera is still moving"
            );
        }
        let mut shown_at = None;
        for frame in 0..120 {
            frame_step(&mut stage);
            if shown_at.is_none() && stage.hud().roll_card.is_some() {
                shown_at = Some(frame);
            }
        }
        assert!(shown_at.is_some(), "and then it shows");
    }

    /// Tonight's village fight: Brakka, the weasel (a third of a hero's height) on his line along
    /// z, and Sylvie nearby. Brakka kills the weasel, then the weasel hits Brakka.
    fn weasel_fight() -> Stage {
        let scene = load_embedded("scan-village").unwrap().unwrap();
        let mut stage = Stage::new(scene);
        let mut weasel = token("weasel", Side::Enemy, Cell::new(3, 1), 4);
        weasel.height_m = 0.6;
        stage.set_tokens(vec![
            token("brakka", Side::Hero, Cell::new(3, 4), 12),
            weasel,
            token("sylvie", Side::Hero, Cell::new(4, 4), 12),
        ]);
        for _ in 0..30 {
            frame_step(&mut stage);
        }
        stage
    }

    fn weasel_hidden_by_brakka(stage: &Stage) -> bool {
        let pose = stage.rig.pose();
        let (brakka, weasel) = (
            body_of_figure(stage, "brakka"),
            body_of_figure(stage, "weasel"),
        );
        let (Some(a), Some(b)) = (
            framing::project_body(&pose, framing::FRAMED_ASPECT, &brakka),
            framing::project_body(&pose, framing::FRAMED_ASPECT, &weasel),
        ) else {
            return true;
        };
        let width = (a.2.min(b.2) - a.0.max(b.0)).max(0.0);
        let height = (a.3.min(b.3) - a.1.max(b.1)).max(0.0);
        let smaller = ((a.2 - a.0) * (a.3 - a.1)).min((b.2 - b.0) * (b.3 - b.1));
        width * height / smaller > framing::MAX_PAIR_COVER
    }

    /// The killing strike and the finishing-blow shot are off the line between the fighters: the
    /// attacker never covers the foe, and the foe is on screen for the whole fall.
    #[test]
    fn a_killing_strike_on_a_small_foe_keeps_it_visible_through_the_fall_and_the_finishing_shot() {
        let mut stage = weasel_fight();
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("brakka"),
                target: TokenId::new("weasel"),
                result: AttackResult::Hit {
                    damage: 13,
                    critical: false,
                    downed: true,
                },
                target_hit_points_after: Some(0),
            })
            .unwrap();
        let mut frames = 0;
        while stage.is_busy() && frames < 1200 {
            frame_step(&mut stage);
            frames += 1;
            let pose = stage.rig.pose();
            let weasel = body_of_figure(&stage, "weasel");
            let centre = framing::project_body(&pose, framing::FRAMED_ASPECT, &weasel)
                .map(|(l, t, r, b)| ((l + r) * 0.5, (t + b) * 0.5));
            let Some((x, y)) = centre else {
                panic!("frame {frames}: the foe is behind the camera");
            };
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "frame {frames}: the foe is off the screen at {x:.2},{y:.2}"
            );
            if !stage.rig.is_panning() {
                assert!(
                    !weasel_hidden_by_brakka(&stage),
                    "frame {frames}: the attacker's body covers the foe"
                );
            }
        }
        assert!(frames < 1200);
    }

    /// The weasel's hit on Brakka: from the first moment the weasel (a small attacker) is on
    /// the screen and so is the target; there is no empty lawn.
    #[test]
    fn a_small_attackers_hit_is_framed_from_the_first_frame() {
        let mut stage = weasel_fight();
        stage.stage(ordinary_hit("weasel", "brakka", 3)).unwrap();
        let mut frames = 0;
        while stage.is_busy() && frames < 900 {
            frame_step(&mut stage);
            frames += 1;
            let pose = stage.rig.pose();
            for id in ["weasel", "brakka"] {
                let body = body_of_figure(&stage, id);
                let centre = framing::project_body(&pose, framing::FRAMED_ASPECT, &body)
                    .map(|(l, t, r, b)| ((l + r) * 0.5, (t + b) * 0.5));
                let Some((x, y)) = centre else {
                    panic!("frame {frames}: {id} is behind the camera");
                };
                assert!(
                    (0.02..=0.98).contains(&x) && (0.02..=0.98).contains(&y),
                    "frame {frames}: {id} is off the screen at {x:.2},{y:.2}"
                );
            }
        }
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
        let mut dropped = stage();
        dropped.stage(killing_blow()).unwrap();
        run(&mut dropped, 0.5);
        assert!(strongest_shift(&mut dropped, "enemy", 4.0) > 5.0);
        // The struck figure's displacement from its cell, every frame, from the start of the
        // staged blow. The stage plays a camera settle and a hold before the lunge, contact at
        // about 1.45 s, then a hit-stop of a few frames and the figure's own recoil; the reaction
        // starts at contact on the stage's simulation clock, so the hit-stop holds it.
        let timeline = |blow: Beat, reactive: bool| {
            install_hooks(reactive);
            let mut stage = stage();
            stage.stage(blow).unwrap();
            let rest = figure(&stage, "enemy").position;
            let mut shifts = Vec::new();
            for _ in 0..120 {
                stage.update(Duration::from_millis(50));
                shifts.push((figure(&stage, "enemy").position - rest).length());
            }
            shifts
        };
        let peak = |shifts: &[f32]| shifts.iter().copied().fold(0.0, f32::max);
        let recoil = timeline(hit(3, false, 2), false);
        let small = timeline(hit(3, false, 2), true);
        let big = timeline(hit(3, true, 2), true);
        // Composed with the stage's recoil, never working against it: at no frame does the
        // figure move less with the reaction than without, and it moves more once the blow lands.
        for (frame, (with, without)) in small.iter().zip(&recoil).enumerate() {
            assert!(
                *with >= *without - 1e-4,
                "frame {frame}: {with} against {without}"
            );
        }
        assert!(
            peak(&small) > peak(&recoil) + 0.05,
            "{} {}",
            peak(&small),
            peak(&recoil)
        );
        assert!(
            peak(&small) < peak(&recoil) + 0.35,
            "an ordinary hit adds a little"
        );
        // The reaction was held through the hit-stop, not skipped: it is still adding to the
        // recoil several frames after contact.
        let contact = recoil.iter().position(|shift| *shift > 0.05).unwrap();
        assert!(small[contact + 4] > recoil[contact + 4] + 0.01);
        // A critical hit of the same size is visibly larger.
        assert!(
            peak(&big) > peak(&small) + 0.7,
            "{} against {}",
            peak(&big),
            peak(&small)
        );
        install_hooks(true);
        // The unstaged path: a view that simply reports fewer hit points reacts as well, and a
        // view that reports more (a heal) does not.
        let mut direct = stage();
        direct.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 4), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 3),
        ]);
        let shift = strongest_shift(&mut direct, "enemy", 1.0);
        assert!(shift > 0.1 && shift < 0.3, "{shift}");
        let mut healed = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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
        let mut stage = stage();
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

    #[test]
    fn a_figure_the_server_moves_walks_around_an_obstacle_and_never_stands_on_a_blocked_cell() {
        let mut stage = layout_stage();
        // The server moves the hero across the field, past the row of crates.
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 1), 12),
            token("enemy", Side::Enemy, Cell::new(3, 2), 5),
        ]);
        assert!(stage.is_busy(), "the move is staged, not teleported");
        let start = figure(&stage, "hero").position;
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
        let end = figure(&stage, "hero").position;
        assert!((end - stage.grid().cell_centre(Cell::new(3, 1))).length() < 0.01);
        assert!((end - start).length() > 1.0);
        assert!(cells.len() > 3);
        assert!(!stage.is_busy());
    }

    #[test]
    fn an_attack_leaves_every_figure_exactly_where_the_server_put_it() {
        let mut stage = stage();
        let home = |stage: &Stage, id: &str| {
            let offset =
                figure(stage, id).position - stage.grid().cell_centre(figure_cell(stage, id));
            offset.x.hypot(offset.z)
        };
        stage
            .stage(Beat::Approach {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
            })
            .unwrap();
        run(&mut stage, 1.6);
        assert!(home(&stage, "hero") < 0.01, "the approach moves nobody");
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
        for _ in 0..240 {
            stage.update(Duration::from_millis(25));
            assert!(
                home(&stage, "hero") < 0.75,
                "at most a lunge within its own square"
            );
        }
        assert!(home(&stage, "hero") < 0.01);
        assert_eq!(figure(&stage, "enemy").hit_points.unwrap().current, 3);
    }

    fn figure_cell(stage: &Stage, id: &str) -> Cell {
        stage
            .tokens()
            .find(|token| token.id.0 == id)
            .map(|token| token.cell)
            .unwrap()
    }

    #[test]
    fn one_footstep_sounds_per_cell_the_server_moves_a_figure() {
        let mut stage = stage();
        stage.set_tokens(vec![
            token("hero", Side::Hero, Cell::new(3, 2), 12),
            token("ally", Side::Hero, Cell::new(5, 4), 12),
            token("enemy", Side::Enemy, Cell::new(4, 1), 5),
        ]);
        let heard = sounds_after(&mut stage, 9.0);
        let steps = heard
            .iter()
            .filter(|(_, kind, who)| *kind == SoundKind::Footstep && who == "hero")
            .count();
        let cells = stage
            .grid()
            .path(Cell::new(3, 4), Cell::new(3, 2))
            .map_or(0, |path| path.len() - 1);
        assert_eq!(steps, cells, "one per cell walked");
    }

    fn crowded_stage() -> Stage {
        let scene = load_embedded("scan-riverbed").unwrap().unwrap();
        let mut stage = Stage::new(scene);
        let mut archer = token("archer", Side::Hero, Cell::new(0, 5), 12);
        archer.attack = crate::battle::model::AttackStyle::Ranged;
        stage.set_tokens(vec![
            archer,
            token("dagna", Side::Hero, Cell::new(2, 5), 12),
            token("hilde", Side::Hero, Cell::new(4, 4), 12),
            token("brakka", Side::Hero, Cell::new(6, 5), 12),
            token("goblin-a", Side::Enemy, Cell::new(7, 0), 5),
            token("goblin-b", Side::Enemy, Cell::new(3, 1), 5),
            token("goblin-c", Side::Enemy, Cell::new(5, 2), 5),
        ]);
        for _ in 0..30 {
            frame_step(&mut stage);
        }
        stage
    }

    /// A ranged attack across the whole field with seven figures spread over the grid: attacker
    /// and target are both in frame at rest, and the arrow is on screen the whole time it flies.
    #[test]
    fn a_ranged_attack_across_a_crowded_field_keeps_both_ends_and_the_arrow_in_frame() {
        let mut stage = crowded_stage();
        stage.stage(ordinary_hit("archer", "goblin-a", 3)).unwrap();
        let mut frames = 0;
        let mut arrow_frames = 0;
        let mut settled = 0;
        while stage.is_busy() && frames < 1200 {
            frame_step(&mut stage);
            frames += 1;
            if !stage.rig.is_panning() && stage.hud().active.is_some() {
                // Settled on the shot: both ends are in the safe area.
                if projected_inside_safe(&stage, "archer")
                    && projected_inside_safe(&stage, "goblin-a")
                {
                    settled += 1;
                }
            }
            let frame = stage.frame();
            for arrow in &frame.projectiles {
                arrow_frames += 1;
                let (x, y) = stage
                    .project(arrow.head, framing::FRAMED_ASPECT)
                    .expect("the arrow is in front of the camera");
                assert!(
                    (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                    "frame {frames}: the arrow is off the screen at {x:.2},{y:.2}"
                );
            }
        }
        assert!(
            arrow_frames > 5,
            "the arrow flew and was seen: {arrow_frames}"
        );
        assert!(
            settled > 20,
            "both fighters framed for most of the beat: {settled}"
        );
    }

    /// Every attacker and target pair among seven spread-out figures is framed: both inside the
    /// safe area from the one pose the camera holds.
    #[test]
    fn any_pair_among_seven_spread_figures_is_framed_with_both_inside_the_safe_area() {
        let stage = crowded_stage();
        let ids = [
            "archer", "dagna", "hilde", "brakka", "goblin-a", "goblin-b", "goblin-c",
        ];
        let style = stage.scene().cameras[&ShotName::TurnFocus];
        let home = stage.scene().cameras[&ShotName::Tactical].yaw_deg;
        for attacker in ids {
            for target in ids {
                if attacker == target {
                    continue;
                }
                let others: Vec<framing::Body> = ids
                    .iter()
                    .filter(|id| **id != attacker && **id != target)
                    .map(|id| body_of_figure(&stage, id))
                    .collect();
                let (a, t) = (
                    body_of_figure(&stage, attacker),
                    body_of_figure(&stage, target),
                );
                let shot = framing::action_shot_from(
                    &style,
                    &stage.rig.pose(),
                    &a,
                    Some(&t),
                    &[],
                    &others,
                    framing::FRAMED_ASPECT,
                    Some(home),
                );
                for fighter in [a, t] {
                    assert!(
                        framing::in_safe_area(&shot.pose, framing::FRAMED_ASPECT, &fighter),
                        "{attacker} -> {target}: {shot:?}"
                    );
                }
            }
        }
    }

    /// The server moves a hero across a crowded field: the mover is on the screen for the whole
    /// move, and nobody else moves.
    #[test]
    fn a_server_move_keeps_the_mover_in_frame_for_its_whole_walk() {
        let mut stage = crowded_stage();
        let before: Vec<Vec3> = ["archer", "dagna", "goblin-b"]
            .iter()
            .map(|id| figure(&stage, id).position)
            .collect();
        let mut tokens: Vec<Token> = stage.tokens().cloned().collect();
        let hero = tokens
            .iter_mut()
            .find(|token| token.id.0 == "brakka")
            .unwrap();
        hero.cell = Cell::new(1, 3);
        stage.set_tokens(tokens);
        let mut frames = 0;
        while stage.is_busy() && frames < 900 {
            frame_step(&mut stage);
            frames += 1;
            let pose = stage.rig.pose();
            let body = body_of_figure(&stage, "brakka");
            let centre = framing::project_body(&pose, framing::FRAMED_ASPECT, &body)
                .map(|(l, t, r, b)| ((l + r) * 0.5, (t + b) * 0.5));
            let Some((x, y)) = centre else {
                panic!("frame {frames}: the mover is behind the camera");
            };
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "frame {frames}: the mover is off the screen at {x:.2},{y:.2}"
            );
        }
        assert!(frames < 900);
        let after: Vec<Vec3> = ["archer", "dagna", "goblin-b"]
            .iter()
            .map(|id| figure(&stage, id).position)
            .collect();
        for (a, b) in before.iter().zip(&after) {
            assert!((*a - *b).length() < 0.02, "nobody else moved");
        }
        let arrived = figure(&stage, "brakka").position;
        assert!((arrived - stage.grid().cell_centre(Cell::new(1, 3))).length() < 0.02);
    }

    /// A spoken line: the bubble belongs to the figure named, the camera eases in on it and the
    /// speaker is in frame; ending the line eases back; the stage is never busy for it.
    #[test]
    fn a_spoken_line_opens_over_its_speaker_with_the_camera_easing_in_and_back() {
        let mut stage = stage();
        run(&mut stage, 1.0);
        let before = stage.camera_pose();
        assert!(!stage.speak("Nobody", "Hello.", 1), "not on the field");
        assert!(stage.frame().speech.is_none());
        let name = figure(&stage, "enemy").name.clone();
        assert!(stage.speak(&name, "Hand over the packet.", 7));
        assert!(!stage.is_busy(), "a line does not hold the fight");
        run(&mut stage, 1.5);
        let speech = stage
            .frame()
            .speech
            .map(|s| (s.name.to_owned(), s.text.to_owned(), s.line_id));
        assert_eq!(
            speech,
            Some((name.clone(), "Hand over the packet.".to_owned(), 7))
        );
        let closer = stage.camera_pose();
        assert!(
            closer.distance < before.distance,
            "{} vs {}",
            closer.distance,
            before.distance
        );
        assert!(projected_inside_safe(&stage, "enemy"));
        stage.end_speech(99);
        assert!(
            stage.frame().speech.is_some(),
            "another line's end is ignored"
        );
        stage.end_speech(7);
        assert!(stage.frame().speech.is_none());
        run(&mut stage, 1.5);
        let back = stage.camera_pose();
        assert!((back.distance - before.distance).abs() < 0.05);
        assert!((back.target - before.target).length() < 0.05);
    }

    #[test]
    fn a_line_that_is_never_ended_closes_itself_and_a_new_line_replaces_the_old() {
        let mut stage = stage();
        run(&mut stage, 1.0);
        let before = stage.camera_pose();
        let first = figure(&stage, "enemy").name.clone();
        let second = figure(&stage, "hero").name.clone();
        assert!(stage.speak(&first, "One.", 1));
        run(&mut stage, 0.5);
        assert!(stage.speak(&second, "Two.", 2));
        assert_eq!(stage.frame().speech.map(|s| s.line_id), Some(2));
        run(&mut stage, speech::MAX_LINE_S + 3.0);
        assert!(stage.frame().speech.is_none());
        let back = stage.camera_pose();
        assert!(
            (back.distance - before.distance).abs() < 0.05,
            "the camera returns to where it was before the first line"
        );
    }

    /// A view carrying three enemy turns, each a walk then a melee attack on the same hero, is
    /// staged cue by cue the way the display does it. Strictly in order: the label names the
    /// creature whose beats are playing (never a later one, never the server's active hero until
    /// the queue is drained), each creature stands where it started until its own walk, and no
    /// result is shown before its attacker stands beside its target.
    #[test]
    fn three_enemy_turns_play_one_after_another_each_walking_before_it_attacks() {
        let mut stage = stage();
        let kobolds = [
            ("kobold-1", Cell::new(2, 1), Cell::new(2, 3)),
            ("kobold-2", Cell::new(4, 1), Cell::new(4, 3)),
            ("kobold-3", Cell::new(6, 1), Cell::new(3, 3)),
        ];
        stage.set_tokens(
            [token("hero", Side::Hero, Cell::new(3, 4), 12)]
                .into_iter()
                .chain(kobolds.map(|(id, from, _)| token(id, Side::Enemy, from, 7)))
                .collect(),
        );
        stage.set_active(Some(TokenId::new("hero")));
        let hero = TokenId::new("hero");
        let centre = |cell: Cell, stage: &Stage| stage.grid().cell_centre(cell);
        let beside = stage.grid().cell_m() * 1.6;
        let mut labels: Vec<String> = Vec::new();
        let mut cues = kobolds.iter().peekable();
        let mut waiting = kobolds.len();
        let mut next_cue_at = 0.0_f32;
        let mut clock = 0.0_f32;
        let mut shown_results = 0;
        let mut result_ids: Vec<u64> = Vec::new();
        for _ in 0..4000 {
            // The next outcome's cue arrives once the stage has nothing queued, as the roll's
            // dice finish; the stage is told the server's final active hero all along.
            stage.hold_active(waiting > 0);
            stage.set_active(Some(hero.clone()));
            if clock >= next_cue_at
                && stage.queue.is_empty()
                && stage.playing.is_none()
                && let Some((id, _, to)) = cues.next()
            {
                let kobold = TokenId::new(*id);
                stage
                    .stage(Beat::TurnStart {
                        actor: kobold.clone(),
                    })
                    .unwrap();
                stage.stage_move(&kobold, *to).unwrap();
                stage.set_attack_label(&kobold, "Dagger", "piercing");
                stage
                    .stage(Beat::Approach {
                        attacker: kobold.clone(),
                        target: hero.clone(),
                    })
                    .unwrap();
                stage
                    .stage(Beat::Strike {
                        attacker: kobold,
                        target: hero.clone(),
                        result: AttackResult::Hit {
                            damage: 3,
                            critical: false,
                            downed: false,
                        },
                        target_hit_points_after: Some(9),
                    })
                    .unwrap();
                waiting -= 1;
                next_cue_at = clock + 1.0;
            }
            stage.update(Duration::from_millis(50));
            clock += 0.05;
            let frame = stage.frame();
            let label = frame
                .hud
                .active
                .as_ref()
                .map(|id| id.0.clone())
                .unwrap_or_default();
            if labels.last() != Some(&label) {
                labels.push(label.clone());
            }
            for (id, from, to) in &kobolds {
                let at = frame
                    .figures
                    .iter()
                    .find(|f| f.id.0 == *id)
                    .unwrap()
                    .position;
                if label != *id {
                    // Not its turn yet: it stands on its starting cell; its turn is over: it
                    // stands beside the hero.
                    let started = (at - centre(*from, &stage)).length() < 0.05;
                    let arrived = (at - centre(*to, &stage)).length() < 0.05;
                    assert!(
                        started || arrived,
                        "{id} stands between cells outside its turn"
                    );
                }
            }
            for float in frame.floats.iter().filter(|float| float.age_s < 0.06) {
                if result_ids.contains(&float.id) {
                    continue;
                }
                result_ids.push(float.id);
                shown_results += 1;
                let attacker = labels.last().unwrap();
                let at = frame
                    .figures
                    .iter()
                    .find(|f| f.id.0 == *attacker)
                    .unwrap()
                    .position;
                let target = frame
                    .figures
                    .iter()
                    .find(|f| f.id.0 == "hero")
                    .unwrap()
                    .position;
                assert!(
                    (at - target).length() <= beside,
                    "{attacker}'s result shows while it stands {:.1} m from its target",
                    (at - target).length()
                );
                assert_eq!(float.text, "Dagger \u{b7} 3 piercing");
            }
        }
        assert_eq!(shown_results, 3, "one result for each attack");
        assert_eq!(
            labels,
            ["kobold-1", "kobold-2", "kobold-3", "hero"],
            "the label follows each creature's turn and returns to the server's active hero"
        );
    }

    /// A suspended turn: creature A moved and attacked, creature B moved part of its path and the
    /// server waits on a reaction. B stays where it stood until A's attack is over, then walks to
    /// where the server stopped it, and only then does the prompt name who decides. Graze on a
    /// miss is a named result too.
    #[test]
    fn a_suspended_turn_shows_after_the_attack_before_it_and_names_who_decides() {
        let mut stage = stage();
        let (a, b, hero) = (
            TokenId::new("kobold-1"),
            TokenId::new("kobold-2"),
            TokenId::new("hero"),
        );
        let (a_from, a_to) = (Cell::new(2, 1), Cell::new(2, 3));
        let (b_from, b_stop) = (Cell::new(6, 1), Cell::new(6, 2));
        let tokens = |a_cell: Cell, b_cell: Cell| {
            vec![
                token("hero", Side::Hero, Cell::new(3, 4), 12),
                token("kobold-1", Side::Enemy, a_cell, 7),
                token("kobold-2", Side::Enemy, b_cell, 7),
            ]
        };
        stage.set_tokens(tokens(a_from, b_from));
        stage.set_active(Some(hero.clone()));
        // The view arrives: A's turn is cued, B is held where it stood, the label stays with A.
        stage.hold_active(true);
        stage.stage(Beat::TurnStart { actor: a.clone() }).unwrap();
        stage.stage_move(&a, a_to).unwrap();
        stage.set_attack_label(&a, "Dagger", "piercing, graze");
        stage
            .stage(Beat::Strike {
                attacker: a.clone(),
                target: hero.clone(),
                result: AttackResult::Hit {
                    damage: 3,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(9),
            })
            .unwrap();
        let mut seen_float = None;
        let mut released = false;
        let position = |stage: &Stage, id: &TokenId| {
            stage
                .frame()
                .figures
                .iter()
                .find(|f| f.id == *id)
                .unwrap()
                .position
        };
        let b_start = stage.grid().cell_centre(b_from);
        let b_end = stage.grid().cell_centre(b_stop);
        for _ in 0..4000 {
            stage.update(Duration::from_millis(50));
            if let Some(text) = stage.frame().floats.first() {
                seen_float.get_or_insert_with(|| text.text.clone());
            }
            if !released && stage.queue.is_empty() && stage.playing.is_none() {
                // A's beats are over: the attacks are drained, B's move and the prompt follow.
                assert!(
                    (position(&stage, &b) - b_start).length() < 0.05,
                    "B moved before A's attack was over"
                );
                released = true;
                stage.hold_active(false);
                stage.set_prompt(Some("HERO: Opportunity Attack?".to_owned()));
                stage.set_tokens(tokens(a_to, b_stop));
            }
        }
        assert!(released);
        assert_eq!(
            seen_float.as_deref(),
            Some("Dagger \u{b7} 3 piercing, graze")
        );
        assert!(
            (position(&stage, &b) - b_end).length() < 0.05,
            "B stands where the server stopped it"
        );
        assert_eq!(
            stage.hud().prompt.as_deref(),
            Some("HERO: Opportunity Attack?")
        );
    }

    /// The floating result names the attack and its result, stays up for its whole hold after
    /// rising, and without a name reads as it always did.
    #[test]
    fn the_floating_result_names_the_attack_and_stays_up_long_enough_to_read() {
        let mut stage = stage();
        stage.set_attack_label(&TokenId::new("hero"), "Greatsword", "slashing");
        stage.stage(ordinary_hit("hero", "enemy", 9)).unwrap();
        let mut born_at = None;
        let mut seen_for = 0.0f32;
        for _ in 0..1200 {
            stage.update(Duration::from_millis(20));
            let frame = stage.frame();
            if let Some(text) = frame.floats.first() {
                assert_eq!(text.text, "Greatsword \u{b7} 9 slashing");
                born_at.get_or_insert(stage.clock_s);
                if text.age_s >= float_text::RISE_S {
                    seen_for = text.age_s - float_text::RISE_S;
                }
            }
        }
        assert!(born_at.is_some(), "the result appeared");
        assert!(seen_for >= 2.5, "stayed up {seen_for} s after rising");
        let mut plain = self::stage();
        plain.stage(ordinary_hit("hero", "enemy", 4)).unwrap();
        run(&mut plain, 3.0);
        let mut shown = Vec::new();
        for _ in 0..200 {
            plain.update(Duration::from_millis(20));
            shown.extend(plain.frame().floats.iter().map(|text| text.text.clone()));
        }
        assert!(
            shown.is_empty() || shown.iter().all(|text| text == "4"),
            "{shown:?}"
        );
    }

    #[test]
    fn a_miss_and_a_critical_knockout_are_named_and_a_knockout_stays_longer() {
        let mut stage = stage();
        stage.set_attack_label(&TokenId::new("hero"), "Longbow", "");
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Miss,
                target_hit_points_after: None,
            })
            .unwrap();
        let mut texts = Vec::new();
        for _ in 0..600 {
            stage.update(Duration::from_millis(20));
            for text in stage.frame().floats {
                if !texts.contains(&text.text) {
                    texts.push(text.text.clone());
                }
            }
        }
        assert_eq!(texts, vec!["Longbow \u{b7} miss".to_owned()]);
        let mut stage = self::stage();
        stage.set_attack_label(&TokenId::new("hero"), "Bite", "");
        stage
            .stage(Beat::Attack {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result: AttackResult::Hit {
                    damage: 7,
                    critical: true,
                    downed: true,
                },
                target_hit_points_after: Some(0),
            })
            .unwrap();
        let mut hold = 0.0;
        let mut text = String::new();
        for _ in 0..1500 {
            stage.update(Duration::from_millis(20));
            if let Some(first) = stage.frame().floats.first() {
                hold = first.hold_s;
                text = first.text.clone();
            }
        }
        assert_eq!(text, "Bite \u{b7} 7, critical, down");
        assert_eq!(hold, float_text::HOLD_EMPHASIS_S);
    }
    /// A fight of `heroes` against `enemies` seated by the derived formation, with everything the
    /// first hero can reach (every walkable cell within six) and every foe highlighted.
    fn sized_stage(id: &str, heroes: usize, enemies: usize) -> Option<(Stage, Vec<Cell>)> {
        let scene = load_embedded(id).unwrap().unwrap();
        let formation = crate::battle::derive_formation(&scene.grid, heroes, enemies).ok()?;
        let mut stage = Stage::new(scene);
        stage.establish();
        let mut tokens = Vec::new();
        for (index, cell) in formation.heroes.iter().enumerate() {
            tokens.push(token(&format!("hero{index}"), Side::Hero, *cell, 12));
        }
        for (index, cell) in formation.enemies.iter().enumerate() {
            tokens.push(token(&format!("foe{index}"), Side::Enemy, *cell, 12));
        }
        for token in &mut tokens {
            token.height_m = if token.side == Side::Hero { 1.35 } else { 1.0 };
        }
        stage.set_tokens(tokens);
        let actor = formation.heroes[0];
        let reachable: Vec<Cell> = stage
            .grid()
            .walkable_cells()
            .filter(|cell| {
                (cell.col - actor.col)
                    .abs()
                    .max((cell.row - actor.row).abs())
                    <= 6
            })
            .collect();
        stage.set_tactical_highlights(reachable.clone(), formation.enemies.clone());
        Some((stage, reachable))
    }

    #[test]
    fn a_turn_leans_toward_its_figure_and_is_back_on_the_wide_shot_within_a_second_and_a_half() {
        let (mut stage, _) = sized_stage("scan-village", 3, 3).unwrap();
        run(&mut stage, 2.0);
        let rest = stage.rig.pose();
        stage
            .stage(Beat::TurnStart {
                actor: TokenId::new("hero0"),
            })
            .unwrap();
        run(&mut stage, 0.6);
        let leaning = stage.rig.pose();
        assert!(
            leaning.distance < rest.distance * 0.85,
            "{leaning:?} vs {rest:?}"
        );
        run(&mut stage, 1.0);
        let back = stage.rig.pose();
        assert!(
            (back.distance - rest.distance).abs() < rest.distance * 0.03,
            "{back:?} vs {rest:?}"
        );
        assert!((back.target - rest.target).length() < 0.6);
    }

    #[test]
    fn the_camera_rests_on_a_wide_shot_that_holds_everyone_clear_of_the_hud_on_every_scene() {
        use crate::battle::rest_shot::{REST_AREA, hud_keep_outs, rects_overlap};
        let mut cases = 0;
        for id in crate::battle::embedded_ids() {
            for (heroes, enemies) in [(1, 1), (4, 3), (6, 6)] {
                let Some((mut stage, reachable)) = sized_stage(id, heroes, enemies) else {
                    continue;
                };
                stage
                    .stage(Beat::TurnStart {
                        actor: TokenId::new("hero0"),
                    })
                    .unwrap();
                run(&mut stage, 4.0);
                let pose = stage.rig.pose();
                let keep_outs = hud_keep_outs(heroes, enemies);
                let inside = |rect: (f32, f32, f32, f32), area: (f32, f32, f32, f32)| {
                    rect.0 >= area.0 && rect.1 >= area.1 && rect.2 <= area.2 && rect.3 <= area.3
                };
                for figure in stage.frame().figures {
                    let body = framing::Body {
                        feet: figure.position,
                        height_m: figure.height_m,
                    };
                    let rect = framing::project_body(&pose, framing::FRAMED_ASPECT, &body)
                        .unwrap_or_else(|| panic!("{id} {heroes}v{enemies}: behind the camera"));
                    assert!(
                        inside(rect, framing::SAFE_AREA) && inside(rect, REST_AREA),
                        "{id} {heroes}v{enemies}: {} at {rect:?}",
                        figure.id.0
                    );
                    for keep_out in &keep_outs {
                        assert!(
                            !rects_overlap(rect, *keep_out),
                            "{id} {heroes}v{enemies}: {} {rect:?} under {keep_out:?}",
                            figure.id.0
                        );
                    }
                }
                for cell in &reachable {
                    for corner in crate::battle::rest_shot::cell_corners(
                        stage.grid().cell_centre(*cell),
                        stage.grid().cell_m(),
                    ) {
                        let (x, y) =
                            framing::project_point(&pose, framing::FRAMED_ASPECT, corner).unwrap();
                        assert!(
                            (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                            "{id} {heroes}v{enemies}: a reachable cell leaves the frame"
                        );
                    }
                }
                cases += 1;
            }
        }
        assert!(cases >= 20, "{cases}");
    }

    #[test]
    fn a_move_keeps_the_wide_shot_and_refits_when_the_reachable_cells_change() {
        let (mut stage, _) = sized_stage("scan-village", 2, 3).unwrap();
        run(&mut stage, 2.0);
        stage
            .stage(Beat::TurnStart {
                actor: TokenId::new("hero0"),
            })
            .unwrap();
        run(&mut stage, 3.0);
        let before = stage.rig.pose();
        // The server now offers only a couple of cells: the held region shrinks, the camera follows.
        stage.set_tactical_highlights(vec![Cell::new(2, 3)], vec![Cell::new(2, 2)]);
        run(&mut stage, 2.5);
        let after = stage.rig.pose();
        assert!(after.pitch_deg >= 33.0 && before.pitch_deg >= 33.0);
        assert!(
            (after.distance - before.distance).abs() > 1e-3
                || (after.target - before.target).length() > 1e-3,
            "{before:?} {after:?}"
        );
    }
}
