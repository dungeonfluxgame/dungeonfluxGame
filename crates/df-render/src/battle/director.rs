//! Turns one server fact ("token X attacks token Y, hit, 7 damage, downed") into an ordered
//! timeline of presentation steps: turn-focus shot, the attacker steps along cells to a free
//! cell beside the target, a lunge, then at contact the impact shot with shake, flash and a
//! floating number while the target recoils; a downing blow adds a fall and the finishing-blow
//! cue; otherwise everyone settles back to formation. The director is pure: it reads the grid
//! and the tokens, decides nothing about outcomes, and returns the same timeline for the same
//! input. Timings below are presentation tuning.

use std::time::Duration;

use super::{
    camera::ShotName,
    feel::{self, ImpactClass},
    grid::{Cell, Grid},
    model::{
        AttackResult, AttackStyle, Beat, CameraCommand, CameraFocus, FloatTone, Highlight,
        HighlightKind, InitiativeRoll, Side, Token, TokenId,
    },
    sound::SoundKind,
};

/// The camera settles on a turn's shot over this long, then holds a beat before anything moves.
const FOCUS_PAN: Duration = Duration::from_millis(800);
const FOCUS_HOLD: Duration = Duration::from_millis(150);
/// The slow push-in on the wind-up, which ends before the attack starts: the camera is at rest at
/// contact.
const WINDUP_PUSH: Duration = Duration::from_millis(380);
const STEP: Duration = Duration::from_millis(260);
const SETTLE_PAN: Duration = Duration::from_millis(1300);
const FINISHING_PUSH: Duration = Duration::from_millis(3200);
const FINISHING_HOLD: Duration = Duration::from_millis(3600);
pub const LUNGE: Duration = Duration::from_millis(460);
pub const LUNGE_CONTACT: Duration = Duration::from_millis(170);
pub const RECOIL: Duration = Duration::from_millis(420);
pub const FALL: Duration = Duration::from_millis(900);
pub const HEAL: Duration = Duration::from_millis(1300);
/// The bow is drawn back for this long, then released; the figure settles over `DRAW`.
pub const DRAW_RELEASE: Duration = Duration::from_millis(450);
pub const DRAW: Duration = Duration::from_millis(600);
/// How long the arrow takes from the bow to the target.
pub const FLIGHT: Duration = Duration::from_millis(400);
/// The slowest a long walk is staged, so a far approach does not stall the fight.
const MAX_APPROACH: Duration = Duration::from_millis(1800);
const MIN_STEP: Duration = Duration::from_millis(150);
/// The fall's body lands on the ground this far into the fall animation.
pub const READY: Duration = Duration::from_millis(700);
pub const FALL_LANDS: Duration = Duration::from_millis(560);
/// Extra time the impact shot stays up after a hit that deals damage.
pub const IMPACT_HOLD: Duration = Duration::from_millis(1800);
const FALL_DELAY: Duration = Duration::from_millis(380);
const FINISHING_DELAY: Duration = Duration::from_millis(560);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Anim {
    Lunge {
        toward: TokenId,
    },
    Recoil {
        from: TokenId,
    },
    Dodge {
        from: TokenId,
    },
    Fall,
    Heal,
    /// Lean back drawing a bow toward the target, then snap forward on release.
    Draw {
        toward: TokenId,
    },
    /// The "ready" flourish at a roll-call stop.
    Ready {
        style: ReadyStyle,
    },
}

/// How a combatant shows it is ready at its roll-call stop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadyStyle {
    /// A hero lifts a weapon and settles.
    Brandish,
    /// A creature snarls, lunging up and rocking back.
    Snarl,
    /// The archer nocks an arrow.
    Nock,
}

impl Anim {
    pub fn duration(&self) -> Duration {
        match self {
            Self::Lunge { .. } => LUNGE,
            Self::Recoil { .. } | Self::Dodge { .. } => RECOIL,
            Self::Fall => FALL,
            Self::Heal => HEAL,
            Self::Draw { .. } => DRAW,
            Self::Ready { .. } => READY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashTone {
    Impact,
    Critical,
    Heal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    SetActive(Option<TokenId>),
    Camera {
        command: CameraCommand,
        pan: Duration,
    },
    Highlight(Option<Highlight>),
    /// Walk a token cell by cell from wherever it stands to `cell` (nothing if already there).
    MoveTo {
        token: TokenId,
        cell: Cell,
        per_step: Duration,
    },
    Animate {
        token: TokenId,
        anim: Anim,
    },
    Shake {
        amplitude_px: f32,
        duration: Duration,
    },
    /// A blow lands on `target`: the stage expands it into shake, hit-stop, flash, particles and
    /// the impact event the HUD subscribes to, all scaled by `class` and `share` (the share of the
    /// target's maximum hit points the blow took). Presentation only; the outcome is the server's.
    Impact {
        attacker: TokenId,
        target: TokenId,
        class: ImpactClass,
        share: f32,
        ranged: bool,
    },
    /// A blow was dodged: the camera sways past it instead of shaking.
    Whoosh,
    /// The finishing blow's slow-motion ramp starts.
    SlowMotion,
    Flash {
        tone: FlashTone,
        strength: f32,
    },
    Float {
        token: TokenId,
        text: String,
        tone: FloatTone,
    },
    /// Shows the server-reported hit points on the token at the moment of impact.
    ShowHitPoints {
        token: TokenId,
        current: u32,
    },
    FinishingBlow {
        hero: TokenId,
        enemy: TokenId,
    },
    EndFinishingBlow,
    /// A sound at this exact moment; the stage resolves the voice and surface.
    Sound {
        kind: SoundKind,
        token: TokenId,
    },
    /// Rings the token being aimed at, with its name tag (or clears it).
    Target(Option<TokenId>),
    /// A fast whip of the camera (a cut with a cross-fade under reduced motion).
    Whip {
        command: CameraCommand,
        travel: Duration,
    },
    /// The big banner text ("Roll for initiative", "Round 1"), or none.
    Banner(Option<String>),
    /// Starts an initiative sequence: the tracker empties and refills as the camera visits.
    InitiativeStart,
    /// A d20 appears above a figure and tumbles (appearing already landed under reduced motion).
    InitiativeDie {
        token: TokenId,
        die: u32,
        modifier: i32,
        total: i32,
    },
    InitiativeLand {
        token: TokenId,
    },
    /// Takes the dice down once the roll-call begins.
    ClearDice,
    /// The order badge slams in beside the figure.
    InitiativeBadge {
        token: TokenId,
        order: u32,
        total: i32,
    },
    /// The figure's portrait pops into the round tracker.
    TrackerReveal(TokenId),
    /// Clears the dice and badges and lets the tracker show everyone.
    InitiativeEnd,
    /// The percussive hit of a roll-call stop; `index` raises the pitch, `last` makes it heavy.
    Sting {
        token: TokenId,
        index: u32,
        last: bool,
    },
    /// An arrow in flight from one token to another. A miss flies past the target.
    Projectile {
        from: TokenId,
        to: TokenId,
        hits: bool,
        flight: Duration,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimedStep {
    pub at: Duration,
    pub step: Step,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    pub steps: Vec<TimedStep>,
    pub duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    UnknownToken(TokenId),
}

struct Builder {
    steps: Vec<TimedStep>,
}

impl Builder {
    fn at(&mut self, at: Duration, step: Step) {
        self.steps.push(TimedStep { at, step });
    }

    fn finish(mut self, duration: Duration) -> Timeline {
        // Stable: steps that share an instant keep the order they were added in.
        self.steps.sort_by_key(|step| step.at);
        Timeline {
            steps: self.steps,
            duration,
        }
    }
}

fn find<'a>(tokens: &'a [Token], id: &TokenId) -> Result<&'a Token, PlanError> {
    tokens
        .iter()
        .find(|token| &token.id == id)
        .ok_or_else(|| PlanError::UnknownToken(id.clone()))
}

/// The action framing for `actor` (acting on `target`, standing at `stand` to do it).
fn action_focus(actor: &TokenId, target: Option<&TokenId>, stand: Option<Cell>) -> CameraFocus {
    CameraFocus::Action {
        actor: actor.clone(),
        target: target.cloned(),
        stand,
    }
}

/// The push-in the camera ends the wind-up with: a little closer than the settled shot.
const PUSH_DOLLY: f32 = 1.0 - feel::PUSH_IN_SHARE;

fn camera(shot: ShotName, focus: CameraFocus, dolly: f32) -> CameraCommand {
    CameraCommand { shot, focus, dolly }
}

/// Plans the staging of `beat` against the current formation. `tokens` carry their formation
/// (home) cells; the attacker's steps are planned from home and the stage resolves where it
/// actually stands.
pub fn plan(grid: &Grid, tokens: &[Token], beat: &Beat) -> Result<Timeline, PlanError> {
    plan_with(grid, tokens, beat, false)
}

/// As [`plan`], telling the director whether motion is reduced (the initiative dice then appear
/// already landed and the sequence is shorter). Outcomes never depend on it.
pub fn plan_with(
    grid: &Grid,
    tokens: &[Token],
    beat: &Beat,
    reduced_motion: bool,
) -> Result<Timeline, PlanError> {
    match beat {
        Beat::Initiative { rolls } => plan_initiative(tokens, rolls, reduced_motion),
        Beat::TurnStart { actor } => plan_turn(tokens, actor),
        Beat::Approach { attacker, target } => {
            let (attacker, target) = (find(tokens, attacker)?, find(tokens, target)?);
            let mut builder = Builder { steps: Vec::new() };
            let end = approach(&mut builder, attacker, target, Duration::ZERO);
            Ok(builder.finish(end))
        }
        Beat::Strike {
            attacker,
            target,
            result,
            target_hit_points_after,
        } => {
            let (attacker, target) = (find(tokens, attacker)?, find(tokens, target)?);
            let mut builder = Builder { steps: Vec::new() };
            builder.at(Duration::ZERO, Step::SetActive(Some(attacker.id.clone())));
            let end = strike(
                &mut builder,
                attacker,
                target,
                *result,
                *target_hit_points_after,
                Duration::ZERO,
            );
            Ok(builder.finish(end))
        }
        Beat::Attack {
            attacker,
            target,
            result,
            target_hit_points_after,
        } => {
            let (attacker, target) = (find(tokens, attacker)?, find(tokens, target)?);
            let mut builder = Builder { steps: Vec::new() };
            let ready = approach(&mut builder, attacker, target, Duration::ZERO);
            let end = strike(
                &mut builder,
                attacker,
                target,
                *result,
                *target_hit_points_after,
                ready + FOCUS_HOLD,
            );
            Ok(builder.finish(end))
        }
        Beat::Move { actor, from, to } => plan_move(grid, tokens, actor, *from, *to),
        Beat::Heal {
            source,
            target,
            amount,
            target_hit_points_after,
        } => plan_heal(tokens, source, target, *amount, *target_hit_points_after),
    }
}

/// Gap between one combatant's die and the next, and how long a die tumbles.
const DICE_STAGGER: Duration = Duration::from_millis(280);
const DICE_TUMBLE: Duration = Duration::from_millis(800);
/// Under reduced motion the dice appear landed, one after another.
const DICE_STAGGER_REDUCED: Duration = Duration::from_millis(160);
const DICE_READ: Duration = Duration::from_millis(700);
/// One roll-call stop is about a second; the whole roll-call never runs past this.
pub const STOP: Duration = Duration::from_millis(1000);
pub const MAX_ROLL_CALL: Duration = Duration::from_millis(7000);
const MIN_STOP: Duration = Duration::from_millis(500);
/// Combatants beyond this many are visited together, as one sweep, instead of one by one.
pub const MAX_INDIVIDUAL_STOPS: usize = 6;
const ROUND_BANNER: Duration = Duration::from_millis(1100);

/// The start-of-fight sequence: the establishing shot and the banner, a d20 for every combatant
/// in seat order, then the roll-call (the camera whips to each combatant in the server's turn
/// order, a badge slams in, a flourish plays and the tracker fills), a pull-back, the "Round 1"
/// banner and the first turn-focus shot. The order is exactly `rolls`; nothing is re-sorted.
fn plan_initiative(
    tokens: &[Token],
    rolls: &[InitiativeRoll],
    reduced_motion: bool,
) -> Result<Timeline, PlanError> {
    let mut builder = Builder { steps: Vec::new() };
    for roll in rolls {
        find(tokens, &roll.token)?;
    }
    let Some(first) = rolls.first() else {
        return Ok(builder.finish(Duration::ZERO));
    };
    builder.at(Duration::ZERO, Step::InitiativeStart);
    builder.at(Duration::ZERO, Step::SetActive(None));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: Duration::from_millis(900),
        },
    );
    builder.at(
        Duration::from_millis(500),
        Step::Banner(Some("Roll for initiative".to_owned())),
    );

    // The dice: seat order is the order of `tokens`.
    let dice_start = Duration::from_millis(1000);
    let (stagger, tumble) = if reduced_motion {
        (DICE_STAGGER_REDUCED, Duration::ZERO)
    } else {
        (DICE_STAGGER, DICE_TUMBLE)
    };
    let seated: Vec<&InitiativeRoll> = tokens
        .iter()
        .filter_map(|token| rolls.iter().find(|roll| roll.token == token.id))
        .collect();
    let mut all_landed = dice_start;
    for (index, roll) in seated.iter().enumerate() {
        let appears = dice_start + stagger * index as u32;
        let lands = appears + tumble;
        builder.at(
            appears,
            Step::InitiativeDie {
                token: roll.token.clone(),
                die: roll.die,
                modifier: roll.modifier,
                total: roll.total,
            },
        );
        if !reduced_motion {
            builder.at(
                appears,
                Step::Sound {
                    kind: SoundKind::DiceRattle,
                    token: roll.token.clone(),
                },
            );
            builder.at(
                lands,
                Step::InitiativeLand {
                    token: roll.token.clone(),
                },
            );
        }
        builder.at(
            lands,
            Step::Sound {
                kind: SoundKind::DiceLand,
                token: roll.token.clone(),
            },
        );
        all_landed = all_landed.max(lands);
    }

    // The roll-call, in the server's order.
    let roll_call = all_landed + DICE_READ;
    builder.at(roll_call, Step::Banner(None));
    builder.at(roll_call, Step::ClearDice);
    let individual = rolls.len().min(MAX_INDIVIDUAL_STOPS);
    let grouped = &rolls[individual..];
    let stops = individual + usize::from(!grouped.is_empty());
    let per_stop = (MAX_ROLL_CALL / stops as u32).clamp(MIN_STOP, STOP);
    let travel = (per_stop * 3 / 10).min(Duration::from_millis(350));
    let ready_for = |token: &Token| {
        if token.side == Side::Enemy {
            ReadyStyle::Snarl
        } else if token.attack == AttackStyle::Ranged {
            ReadyStyle::Nock
        } else {
            ReadyStyle::Brandish
        }
    };
    for (index, roll) in rolls.iter().take(individual).enumerate() {
        let at = roll_call + per_stop * index as u32;
        let arrive = at + travel;
        let token = find(tokens, &roll.token)?;
        builder.at(
            at,
            Step::Whip {
                command: camera(
                    ShotName::Victory,
                    CameraFocus::Token(roll.token.clone()),
                    1.15,
                ),
                travel,
            },
        );
        builder.at(
            arrive,
            Step::Animate {
                token: roll.token.clone(),
                anim: Anim::Ready {
                    style: ready_for(token),
                },
            },
        );
        builder.at(
            arrive,
            Step::InitiativeBadge {
                token: roll.token.clone(),
                order: index as u32 + 1,
                total: roll.total,
            },
        );
        builder.at(arrive, Step::TrackerReveal(roll.token.clone()));
        builder.at(
            arrive,
            Step::Sting {
                token: roll.token.clone(),
                index: index as u32,
                last: index + 1 == stops,
            },
        );
    }
    if let (Some(head), Some(tail)) = (grouped.first(), grouped.last()) {
        // One sweep over everyone left: a wide shot between the first and last of them.
        let at = roll_call + per_stop * individual as u32;
        let arrive = at + travel;
        builder.at(
            at,
            Step::Whip {
                command: camera(
                    ShotName::Impact,
                    CameraFocus::Between(head.token.clone(), tail.token.clone()),
                    1.5,
                ),
                travel,
            },
        );
        for (offset, roll) in grouped.iter().enumerate() {
            let token = find(tokens, &roll.token)?;
            builder.at(
                arrive,
                Step::Animate {
                    token: roll.token.clone(),
                    anim: Anim::Ready {
                        style: ready_for(token),
                    },
                },
            );
            builder.at(
                arrive,
                Step::InitiativeBadge {
                    token: roll.token.clone(),
                    order: (individual + offset) as u32 + 1,
                    total: roll.total,
                },
            );
            builder.at(arrive, Step::TrackerReveal(roll.token.clone()));
        }
        builder.at(
            arrive,
            Step::Sting {
                token: head.token.clone(),
                index: individual as u32,
                last: true,
            },
        );
    }

    // Pull back, then the round banner and the first turn.
    let over = roll_call + per_stop * stops as u32;
    builder.at(
        over,
        Step::Whip {
            command: camera(ShotName::Tactical, CameraFocus::Scene, 1.0),
            travel: Duration::from_millis(350),
        },
    );
    builder.at(over + Duration::from_millis(150), Step::InitiativeEnd);
    let banner = over + Duration::from_millis(450);
    builder.at(banner, Step::Banner(Some("Round 1".to_owned())));
    let turn = banner + ROUND_BANNER;
    builder.at(turn, Step::Banner(None));
    builder.at(turn, Step::SetActive(Some(first.token.clone())));
    let first_token = find(tokens, &first.token)?;
    builder.at(
        turn,
        Step::Camera {
            command: camera(
                ShotName::TurnFocus,
                CameraFocus::Token(first.token.clone()),
                1.0,
            ),
            pan: FOCUS_PAN,
        },
    );
    builder.at(
        turn,
        Step::Highlight(Some(Highlight {
            kind: HighlightKind::Move,
            cells: vec![first_token.cell],
        })),
    );
    Ok(builder.finish(turn + FOCUS_PAN))
}

fn plan_turn(tokens: &[Token], actor: &TokenId) -> Result<Timeline, PlanError> {
    let token = find(tokens, actor)?;
    let mut builder = Builder { steps: Vec::new() };
    builder.at(Duration::ZERO, Step::SetActive(Some(actor.clone())));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(ShotName::TurnFocus, action_focus(actor, None, None), 1.0),
            pan: FOCUS_PAN,
        },
    );
    builder.at(
        Duration::ZERO,
        Step::Highlight(Some(Highlight {
            kind: HighlightKind::Move,
            cells: vec![token.cell],
        })),
    );
    Ok(builder.finish(FOCUS_PAN))
}

/// Turn-focus shot on the attacker and its target, then a slow push-in on the wind-up. Nobody
/// walks: figures stand where the server put them and move only when the server moves them (see
/// [`plan_move`]). Returns when the camera is set and the push-in is done.
fn approach(builder: &mut Builder, attacker: &Token, target: &Token, base: Duration) -> Duration {
    builder.at(base, Step::SetActive(Some(attacker.id.clone())));
    let framing = action_focus(&attacker.id, Some(&target.id), None);
    builder.at(
        base,
        Step::Camera {
            command: camera(ShotName::TurnFocus, framing.clone(), 1.0),
            pan: FOCUS_PAN,
        },
    );
    builder.at(base, Step::Target(Some(target.id.clone())));
    let end = base + FOCUS_PAN;
    // A slow push-in on the wind-up that is finished before the blow starts.
    builder.at(
        end,
        Step::Camera {
            command: camera(ShotName::TurnFocus, framing, PUSH_DOLLY),
            pan: WINDUP_PUSH,
        },
    );
    end + WINDUP_PUSH
}

/// The server moved `actor` to `to`: the camera frames where it starts and where it ends, the
/// figure walks the path of cells (a straight eased glide when no path is open), and the camera
/// settles back to the wide shot after a moment. This is movement the server resolved.
fn plan_move(
    grid: &Grid,
    tokens: &[Token],
    actor: &TokenId,
    from: Cell,
    to: Cell,
) -> Result<Timeline, PlanError> {
    find(tokens, actor)?;
    let mut builder = Builder { steps: Vec::new() };
    builder.at(Duration::ZERO, Step::SetActive(Some(actor.clone())));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(
                ShotName::TurnFocus,
                action_focus(actor, None, Some(to)),
                1.0,
            ),
            pan: FOCUS_PAN,
        },
    );
    let steps = grid
        .path(from, to)
        .map_or(1, |path| path.len().saturating_sub(1)) as u32;
    let per_step = walk_pace(steps.max(1));
    let start = FOCUS_PAN + FOCUS_HOLD;
    builder.at(
        start,
        Step::MoveTo {
            token: actor.clone(),
            cell: to,
            per_step,
        },
    );
    let arrived = start + per_step * steps.max(1);
    builder.at(
        arrived + Duration::from_millis(400),
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: SETTLE_PAN,
        },
    );
    Ok(builder.finish(arrived + Duration::from_millis(400) + SETTLE_PAN))
}

/// Time per cell for a walk of `steps` cells: the usual pace, quickened (to a floor) so that a
/// long approach across a big field stays under [`MAX_APPROACH`].
fn walk_pace(steps: u32) -> Duration {
    if steps == 0 || STEP * steps <= MAX_APPROACH {
        return STEP;
    }
    (MAX_APPROACH / steps).max(MIN_STEP)
}

/// Lunge or bow, contact (impact shot, shake, flash, number, recoil) and then either the fall
/// and the finishing cue, or everyone settling back to formation. Returns when it is over.
#[allow(clippy::too_many_arguments)]
fn strike(
    builder: &mut Builder,
    attacker: &Token,
    target: &Token,
    result: AttackResult,
    hit_points_after: Option<u32>,
    base: Duration,
) -> Duration {
    let sound = |builder: &mut Builder, at, kind, token: &Token| {
        builder.at(
            at,
            Step::Sound {
                kind,
                token: token.id.clone(),
            },
        );
    };
    builder.at(
        base,
        Step::Highlight(Some(Highlight {
            kind: HighlightKind::Attack,
            cells: vec![target.cell],
        })),
    );
    let ranged = attacker.attack == AttackStyle::Ranged;
    let hits = matches!(result, AttackResult::Hit { .. });
    let (contact, mut end) = if ranged {
        let release = base + DRAW_RELEASE;
        builder.at(
            base,
            Step::Animate {
                token: attacker.id.clone(),
                anim: Anim::Draw {
                    toward: target.id.clone(),
                },
            },
        );
        sound(builder, base, SoundKind::BowDraw, attacker);
        sound(builder, release, SoundKind::BowRelease, attacker);
        sound(builder, release, SoundKind::Effort, attacker);
        sound(
            builder,
            release + Duration::from_millis(40),
            SoundKind::ArrowFlight,
            attacker,
        );
        builder.at(
            release,
            Step::Projectile {
                from: attacker.id.clone(),
                to: target.id.clone(),
                hits,
                flight: FLIGHT,
            },
        );
        let contact = release + FLIGHT;
        (contact, contact + (LUNGE - LUNGE_CONTACT))
    } else {
        builder.at(
            base,
            Step::Animate {
                token: attacker.id.clone(),
                anim: Anim::Lunge {
                    toward: target.id.clone(),
                },
            },
        );
        sound(builder, base, SoundKind::SwordSwing, attacker);
        sound(
            builder,
            base + Duration::from_millis(30),
            SoundKind::Effort,
            attacker,
        );
        (base + LUNGE_CONTACT, base + LUNGE)
    };
    builder.at(contact, Step::Target(None));
    // The camera does not move at contact: the action shot already holds both fighters, so the
    // hit-stop and the shake land on a stable frame.
    let finishing = matches!(result, AttackResult::Hit { downed: true, .. })
        && target.side == Side::Enemy
        && attacker.side == Side::Hero;
    if finishing {
        // Time slows into the blow.
        builder.at(base, Step::SlowMotion);
    }
    match result {
        AttackResult::Miss => {
            builder.at(contact, Step::Whoosh);
            builder.at(
                contact,
                Step::Animate {
                    token: target.id.clone(),
                    anim: Anim::Dodge {
                        from: attacker.id.clone(),
                    },
                },
            );
            builder.at(
                contact,
                Step::Float {
                    token: target.id.clone(),
                    text: "Miss".to_owned(),
                    tone: FloatTone::Miss,
                },
            );
            let miss_sound = if ranged {
                SoundKind::ArrowMiss
            } else {
                SoundKind::Parry
            };
            sound(builder, contact, miss_sound, target);
        }
        AttackResult::Hit {
            damage,
            critical,
            downed,
        } => {
            let maximum = target.hit_points.map_or(0, |hp| hp.maximum);
            let class = feel::classify(damage, maximum, critical, downed);
            builder.at(
                contact,
                Step::Impact {
                    attacker: attacker.id.clone(),
                    target: target.id.clone(),
                    class,
                    share: feel::damage_share(damage, maximum),
                    ranged,
                },
            );
            builder.at(
                contact,
                Step::Float {
                    token: target.id.clone(),
                    text: damage.to_string(),
                    tone: if critical {
                        FloatTone::Critical
                    } else {
                        FloatTone::Damage
                    },
                },
            );
            builder.at(
                contact,
                Step::Animate {
                    token: target.id.clone(),
                    anim: Anim::Recoil {
                        from: attacker.id.clone(),
                    },
                },
            );
            let impact_sound = if ranged {
                SoundKind::ArrowImpact
            } else {
                SoundKind::BladeHit
            };
            sound(builder, contact, impact_sound, target);
            sound(
                builder,
                contact + Duration::from_millis(60),
                SoundKind::Pain,
                target,
            );
            if let Some(current) = hit_points_after {
                builder.at(
                    contact,
                    Step::ShowHitPoints {
                        token: target.id.clone(),
                        current,
                    },
                );
            }
            if downed {
                let fall = contact + FALL_DELAY;
                builder.at(
                    fall,
                    Step::Animate {
                        token: target.id.clone(),
                        anim: Anim::Fall,
                    },
                );
                sound(builder, fall, SoundKind::Down, target);
                sound(builder, fall + FALL_LANDS, SoundKind::BodyFall, target);
                if target.side == Side::Enemy && attacker.side == Side::Hero {
                    let cue = fall + FINISHING_DELAY;
                    builder.at(
                        cue,
                        Step::FinishingBlow {
                            hero: attacker.id.clone(),
                            enemy: target.id.clone(),
                        },
                    );
                    // The foe's fall is the subject: both are framed, off the line between them, so
                    // the falling foe is on screen for its whole duration.
                    builder.at(
                        cue,
                        Step::Camera {
                            command: camera(
                                ShotName::Victory,
                                action_focus(&attacker.id, Some(&target.id), None),
                                0.92,
                            ),
                            pan: FINISHING_PUSH,
                        },
                    );
                    let over = cue + FINISHING_HOLD;
                    builder.at(over, Step::EndFinishingBlow);
                    return over;
                }
                builder.at(
                    fall,
                    Step::Camera {
                        command: camera(
                            ShotName::Ko,
                            action_focus(&attacker.id, Some(&target.id), None),
                            1.0,
                        ),
                        pan: SETTLE_PAN,
                    },
                );
                return fall + FALL;
            }
        }
    }

    // Settle: everyone returns to formation and the camera widens. After a hit the impact shot
    // is held so the number and the hit-point drop are read on a TV across the room.
    end += Duration::from_millis(220);
    if hits {
        end += IMPACT_HOLD;
    }
    builder.at(end, Step::Highlight(None));
    builder.at(
        end + Duration::from_millis(200),
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: SETTLE_PAN,
        },
    );
    end + SETTLE_PAN + Duration::from_millis(200)
}

fn plan_heal(
    tokens: &[Token],
    source_id: &TokenId,
    target_id: &TokenId,
    amount: u32,
    hit_points_after: Option<u32>,
) -> Result<Timeline, PlanError> {
    find(tokens, source_id)?;
    find(tokens, target_id)?;
    let mut builder = Builder { steps: Vec::new() };
    builder.at(Duration::ZERO, Step::SetActive(Some(source_id.clone())));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(
                ShotName::TurnFocus,
                action_focus(
                    source_id,
                    (source_id != target_id).then_some(target_id),
                    None,
                ),
                1.0,
            ),
            pan: FOCUS_PAN,
        },
    );
    let now = FOCUS_PAN + FOCUS_HOLD;
    builder.at(
        now,
        Step::Animate {
            token: target_id.clone(),
            anim: Anim::Heal,
        },
    );
    builder.at(
        now,
        Step::Sound {
            kind: SoundKind::HealBreath,
            token: target_id.clone(),
        },
    );
    builder.at(
        now,
        Step::Flash {
            tone: FlashTone::Heal,
            strength: 0.45,
        },
    );
    builder.at(
        now,
        Step::Float {
            token: target_id.clone(),
            text: format!("+{amount}"),
            tone: FloatTone::Heal,
        },
    );
    if let Some(current) = hit_points_after {
        builder.at(
            now,
            Step::ShowHitPoints {
                token: target_id.clone(),
                current,
            },
        );
    }
    let end = now + HEAL;
    builder.at(
        end,
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: SETTLE_PAN,
        },
    );
    Ok(builder.finish(end + SETTLE_PAN))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: &str, side: Side, cell: Cell) -> Token {
        Token {
            id: TokenId::new(id),
            side,
            name: id.to_owned(),
            cell,
            height_m: 1.8,
            figure: None,
            portrait: None,
            hit_points: None,
            statuses: Vec::new(),
            attack: AttackStyle::Melee,
            voice: None,
        }
    }

    fn field() -> (Grid, Vec<Token>) {
        let grid = Grid::open_except([0.0, 0.0], 1.5, 8, 6, &[]).unwrap();
        let tokens = vec![
            token("hero", Side::Hero, Cell::new(3, 4)),
            token("ally", Side::Hero, Cell::new(5, 4)),
            token("enemy", Side::Enemy, Cell::new(4, 1)),
        ];
        (grid, tokens)
    }

    fn attack(result: AttackResult) -> Beat {
        Beat::Attack {
            attacker: TokenId::new("hero"),
            target: TokenId::new("enemy"),
            result,
            target_hit_points_after: Some(0),
        }
    }

    /// A readable label per step, in timeline order.
    fn names(timeline: &Timeline) -> Vec<&'static str> {
        timeline
            .steps
            .iter()
            .map(|timed| match &timed.step {
                Step::SetActive(_) => "active",
                Step::Camera { command, .. } => match command.shot {
                    ShotName::TurnFocus => "focus",
                    ShotName::Impact => "impact-shot",
                    ShotName::Victory => "victory-shot",
                    ShotName::CombatEst => "settle-shot",
                    ShotName::Ko => "ko-shot",
                    ShotName::Tactical => "tactical-shot",
                },
                Step::Highlight(_) => "highlight",
                Step::MoveTo { .. } => "walk",
                Step::Animate { anim, .. } => match anim {
                    Anim::Lunge { .. } => "lunge",
                    Anim::Recoil { .. } => "recoil",
                    Anim::Dodge { .. } => "dodge",
                    Anim::Fall => "fall",
                    Anim::Heal => "heal",
                    Anim::Draw { .. } => "draw",
                    Anim::Ready { .. } => "ready",
                },
                Step::Shake { .. } => "shake",
                Step::Impact { .. } => "impact",
                Step::Whoosh => "whoosh",
                Step::SlowMotion => "slow-motion",
                Step::Flash { .. } => "flash",
                Step::Float { .. } => "float",
                Step::ShowHitPoints { .. } => "hp",
                Step::FinishingBlow { .. } => "finishing",
                Step::EndFinishingBlow => "finishing-end",
                Step::Sound { kind, .. } => match kind {
                    SoundKind::SwordSwing => "snd-swing",
                    SoundKind::BladeHit => "snd-hit",
                    SoundKind::Parry => "snd-parry",
                    SoundKind::BowDraw => "snd-bow-draw",
                    SoundKind::BowRelease => "snd-bow-release",
                    SoundKind::ArrowFlight => "snd-arrow-flight",
                    SoundKind::ArrowImpact => "snd-arrow-impact",
                    SoundKind::ArrowMiss => "snd-arrow-miss",
                    SoundKind::Effort => "snd-effort",
                    SoundKind::Pain => "snd-pain",
                    SoundKind::Down => "snd-down",
                    SoundKind::BodyFall => "snd-body-fall",
                    SoundKind::HealBreath => "snd-heal",
                    SoundKind::Footstep => "snd-step",
                    SoundKind::DiceRattle => "snd-dice-rattle",
                    SoundKind::DiceLand => "snd-dice-land",
                    SoundKind::InitiativeStop => "snd-stop",
                    SoundKind::InitiativeFinal => "snd-final",
                },
                Step::Whip { .. } => "whip",
                Step::Banner(Some(text)) if text.starts_with("Roll") => "banner-roll",
                Step::Banner(Some(_)) => "banner-round",
                Step::Banner(None) => "banner-off",
                Step::InitiativeStart => "init-start",
                Step::InitiativeDie { .. } => "die",
                Step::InitiativeLand { .. } => "die-land",
                Step::ClearDice => "clear-dice",
                Step::InitiativeBadge { .. } => "badge",
                Step::TrackerReveal(_) => "tracker",
                Step::InitiativeEnd => "init-end",
                Step::Sting { .. } => "sting",
                Step::Target(Some(_)) => "target",
                Step::Target(None) => "untarget",
                Step::Projectile { .. } => "arrow",
            })
            .collect()
    }

    fn position(names: &[&str], wanted: &str) -> usize {
        names
            .iter()
            .position(|name| *name == wanted)
            .unwrap_or_else(|| panic!("{wanted} missing from {names:?}"))
    }

    #[test]
    fn a_downing_hit_runs_focus_step_lunge_impact_fall_then_the_finishing_cue_in_order() {
        let (grid, tokens) = field();
        let timeline = plan(
            &grid,
            &tokens,
            &attack(AttackResult::Hit {
                damage: 9,
                critical: false,
                downed: true,
            }),
        )
        .unwrap();
        let names = names(&timeline);
        let order = [
            "focus",
            "lunge",
            "impact",
            "float",
            "recoil",
            "fall",
            "finishing",
        ];
        let positions: Vec<usize> = order.iter().map(|name| position(&names, name)).collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "out of order: {names:?}"
        );
        let times: Vec<Duration> = timeline.steps.iter().map(|step| step.at).collect();
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(timeline.duration >= *times.last().unwrap());
    }

    fn moves(timeline: &Timeline) -> Vec<(Cell, Duration)> {
        timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::MoveTo { cell, per_step, .. } => Some((*cell, *per_step)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nobody_walks_to_attack_and_nobody_walks_back() {
        // Figures stand where the server put them: an attack plans no movement at all, from near
        // or far, and an attack from far away does not walk up.
        let (grid, tokens) = field();
        for result in [
            AttackResult::Miss,
            AttackResult::Hit {
                damage: 3,
                critical: false,
                downed: false,
            },
            AttackResult::Hit {
                damage: 9,
                critical: false,
                downed: true,
            },
        ] {
            let timeline = plan(&grid, &tokens, &attack(result)).unwrap();
            assert!(moves(&timeline).is_empty(), "{result:?}");
            assert!(!names(&timeline).contains(&"walk"));
        }
    }

    #[test]
    fn a_server_move_plans_the_path_with_the_camera_on_both_ends_then_settles() {
        let (grid, tokens) = field();
        let timeline = plan(
            &grid,
            &tokens,
            &Beat::Move {
                actor: TokenId::new("hero"),
                from: Cell::new(3, 4),
                to: Cell::new(6, 2),
            },
        )
        .unwrap();
        let names = names(&timeline);
        assert_eq!(
            names,
            vec!["active", "focus", "highlight", "walk", "settle-shot"]
                .into_iter()
                .filter(|name| names.contains(name))
                .collect::<Vec<_>>()
        );
        let moves = moves(&timeline);
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].0, Cell::new(6, 2));
        // The move starts after the camera has settled and lasts as long as the walk takes.
        let walk_at = timeline
            .steps
            .iter()
            .find(|timed| matches!(timed.step, Step::MoveTo { .. }))
            .unwrap()
            .at;
        assert!(walk_at >= FOCUS_PAN);
        assert!(timeline.duration >= walk_at + moves[0].1 * 3);
    }

    #[test]
    fn approach_and_strike_together_match_the_whole_attack_in_order() {
        let (grid, tokens) = field();
        let result = AttackResult::Hit {
            damage: 4,
            critical: false,
            downed: false,
        };
        let approach_only = plan(
            &grid,
            &tokens,
            &Beat::Approach {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
            },
        )
        .unwrap();
        let strike_only = plan(
            &grid,
            &tokens,
            &Beat::Strike {
                attacker: TokenId::new("hero"),
                target: TokenId::new("enemy"),
                result,
                target_hit_points_after: Some(3),
            },
        )
        .unwrap();
        let first = names(&approach_only);
        assert_eq!(first, vec!["active", "focus", "target", "focus"]);
        assert!(!first.contains(&"lunge"));
        let second = names(&strike_only);
        assert!(position(&second, "lunge") < position(&second, "impact"));
        assert!(!second.contains(&"focus"));
        assert!(moves(&strike_only).is_empty(), "the strike moves nobody");
        assert!(approach_only.duration < Duration::from_millis(1200));
    }

    #[test]
    fn a_miss_dodges_without_shake_flash_or_fall() {
        let (grid, tokens) = field();
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        let names = names(&timeline);
        assert!(names.contains(&"dodge") && names.contains(&"float"));
        assert!(
            names.contains(&"whoosh"),
            "a miss sways the camera past: {names:?}"
        );
        for absent in ["shake", "flash", "impact", "recoil", "fall", "finishing"] {
            assert!(!names.contains(&absent), "{absent} in {names:?}");
        }
    }

    #[test]
    fn a_critical_hit_lands_as_a_heavier_impact_than_an_ordinary_one() {
        let (grid, tokens) = field();
        let strength = |critical| {
            let timeline = plan(
                &grid,
                &tokens,
                &attack(AttackResult::Hit {
                    damage: 5,
                    critical,
                    downed: false,
                }),
            )
            .unwrap();
            timeline
                .steps
                .iter()
                .find_map(|timed| match timed.step {
                    Step::Impact { class, .. } => Some(class),
                    _ => None,
                })
                .unwrap()
        };
        assert!(strength(true) > strength(false));
        assert_eq!(strength(true), ImpactClass::Critical);
    }

    #[test]
    fn a_downed_hero_falls_without_a_finishing_blow_card() {
        let (grid, tokens) = field();
        let beat = Beat::Attack {
            attacker: TokenId::new("enemy"),
            target: TokenId::new("hero"),
            result: AttackResult::Hit {
                damage: 4,
                critical: false,
                downed: true,
            },
            target_hit_points_after: Some(1),
        };
        let names = names(&plan(&grid, &tokens, &beat).unwrap());
        assert!(names.contains(&"fall"));
        assert!(!names.contains(&"finishing"));
        assert!(names.contains(&"ko-shot"));
    }

    #[test]
    fn heal_focuses_the_healer_then_shimmers_the_target() {
        let (grid, tokens) = field();
        let beat = Beat::Heal {
            source: TokenId::new("hero"),
            target: TokenId::new("hero"),
            amount: 7,
            target_hit_points_after: Some(12),
        };
        let names = names(&plan(&grid, &tokens, &beat).unwrap());
        assert!(position(&names, "focus") < position(&names, "heal"));
        assert!(names.contains(&"flash") && names.contains(&"float"));
    }

    #[test]
    fn an_unknown_token_is_a_typed_planning_error() {
        let (grid, tokens) = field();
        let beat = Beat::TurnStart {
            actor: TokenId::new("ghost"),
        };
        assert_eq!(
            plan(&grid, &tokens, &beat),
            Err(PlanError::UnknownToken(TokenId::new("ghost")))
        );
    }

    #[test]
    fn planning_is_deterministic() {
        let (grid, tokens) = field();
        let beat = attack(AttackResult::Hit {
            damage: 6,
            critical: false,
            downed: false,
        });
        assert_eq!(plan(&grid, &tokens, &beat), plan(&grid, &tokens, &beat));
    }

    fn ranged_field() -> (Grid, Vec<Token>) {
        let (grid, mut tokens) = field();
        tokens[0].attack = AttackStyle::Ranged;
        (grid, tokens)
    }

    fn times_of(timeline: &Timeline, wanted: &str) -> Vec<Duration> {
        let labels = names(timeline);
        timeline
            .steps
            .iter()
            .zip(labels)
            .filter(|(_, label)| *label == wanted)
            .map(|(timed, _)| timed.at)
            .collect()
    }

    #[test]
    fn a_ranged_attacker_draws_releases_flies_and_only_then_lands_the_blow() {
        let (grid, tokens) = ranged_field();
        let timeline = plan(
            &grid,
            &tokens,
            &attack(AttackResult::Hit {
                damage: 4,
                critical: false,
                downed: false,
            }),
        )
        .unwrap();
        let labels = names(&timeline);
        assert!(
            !labels.contains(&"walk")
                || moves(&timeline)
                    .iter()
                    .all(|(cell, _)| *cell == Cell::new(3, 4))
        );
        let draw = times_of(&timeline, "draw")[0];
        let release = times_of(&timeline, "snd-bow-release")[0];
        let flight = times_of(&timeline, "arrow")[0];
        let impact = times_of(&timeline, "snd-arrow-impact")[0];
        let number = times_of(&timeline, "float")[0];
        assert!(draw < release && release == flight && flight < impact);
        assert_eq!(
            impact - release,
            FLIGHT,
            "the blow lands when the arrow arrives"
        );
        assert_eq!(number, impact);
        assert_eq!(times_of(&timeline, "snd-bow-draw")[0], draw);
        assert!(times_of(&timeline, "snd-arrow-flight")[0] >= release);
        assert!(times_of(&timeline, "snd-pain")[0] > impact);
        assert!(times_of(&timeline, "snd-swing").is_empty());
        let flight_ms = FLIGHT.as_millis();
        assert!((350..=450).contains(&flight_ms), "{flight_ms}");
    }

    #[test]
    fn a_ranged_miss_sends_the_arrow_past_and_sticks_it() {
        let (grid, tokens) = ranged_field();
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        let labels = names(&timeline);
        assert!(labels.contains(&"snd-arrow-miss") && labels.contains(&"dodge"));
        for absent in ["snd-arrow-impact", "snd-hit", "recoil", "snd-pain"] {
            assert!(!labels.contains(&absent), "{absent} in {labels:?}");
        }
        let arrow = timeline
            .steps
            .iter()
            .find_map(|timed| match &timed.step {
                Step::Projectile { hits, .. } => Some(*hits),
                _ => None,
            })
            .unwrap();
        assert!(!arrow);
    }

    #[test]
    fn a_ranged_attacker_never_walks_up_even_from_far_away() {
        let (grid, mut tokens) = ranged_field();
        tokens[0].cell = Cell::new(0, 5);
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        assert!(
            moves(&timeline)
                .iter()
                .all(|(cell, _)| *cell == Cell::new(0, 5))
        );
    }

    #[test]
    fn a_melee_blow_sounds_swing_effort_then_impact_and_pain_in_order() {
        let (grid, tokens) = field();
        let timeline = plan(
            &grid,
            &tokens,
            &attack(AttackResult::Hit {
                damage: 9,
                critical: false,
                downed: true,
            }),
        )
        .unwrap();
        let labels = names(&timeline);
        let order = [
            "snd-swing",
            "snd-effort",
            "snd-hit",
            "snd-pain",
            "snd-down",
            "snd-body-fall",
        ];
        let positions: Vec<usize> = order.iter().map(|name| position(&labels, name)).collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{labels:?}"
        );
        let swing = times_of(&timeline, "snd-swing")[0];
        let hit = times_of(&timeline, "snd-hit")[0];
        assert_eq!(hit - swing, LUNGE_CONTACT, "the impact sounds at contact");
    }

    #[test]
    fn a_melee_miss_swings_then_parries_without_flesh_sounds() {
        let (grid, tokens) = field();
        let labels = names(&plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap());
        assert!(position(&labels, "snd-swing") < position(&labels, "snd-parry"));
        assert!(!labels.contains(&"snd-hit") && !labels.contains(&"snd-pain"));
    }

    #[test]
    fn the_target_is_ringed_during_the_focus_shot_and_released_at_contact() {
        let (grid, tokens) = field();
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        let labels = names(&timeline);
        assert!(position(&labels, "target") < position(&labels, "lunge"));
        assert!(position(&labels, "lunge") < position(&labels, "untarget"));
        assert_eq!(times_of(&timeline, "target")[0], Duration::ZERO);
    }

    #[test]
    fn a_heal_breathes_at_the_moment_of_the_shimmer() {
        let (grid, tokens) = field();
        let beat = Beat::Heal {
            source: TokenId::new("hero"),
            target: TokenId::new("ally"),
            amount: 3,
            target_hit_points_after: None,
        };
        let timeline = plan(&grid, &tokens, &beat).unwrap();
        assert_eq!(times_of(&timeline, "snd-heal"), times_of(&timeline, "heal"));
    }

    #[test]
    fn a_hit_holds_the_impact_shot_longer_than_a_miss_before_the_camera_widens() {
        let (grid, tokens) = field();
        let widen = |result| {
            let timeline = plan(&grid, &tokens, &attack(result)).unwrap();
            let contact = times_of(&timeline, "float")[0];
            let settle = *times_of(&timeline, "settle-shot").last().unwrap();
            settle - contact
        };
        let hit = widen(AttackResult::Hit {
            damage: 5,
            critical: false,
            downed: false,
        });
        let miss = widen(AttackResult::Miss);
        assert!(hit >= miss + IMPACT_HOLD, "hit {hit:?} vs miss {miss:?}");
        assert!(
            hit >= Duration::from_millis(2400),
            "the number lives 1.4 s and is read: {hit:?}"
        );
    }

    #[test]
    fn a_long_walk_is_quickened_so_the_fight_does_not_stall() {
        assert_eq!(walk_pace(2), STEP);
        let steps = 12;
        let pace = walk_pace(steps);
        assert!(pace < STEP && pace >= MIN_STEP);
        assert!(pace * steps <= MAX_APPROACH + Duration::from_millis(12));
    }

    fn roll(id: &str, die: u32, modifier: i32) -> InitiativeRoll {
        InitiativeRoll {
            token: TokenId::new(id),
            die,
            modifier,
            total: die as i32 + modifier,
        }
    }

    /// Heroes and the goblin on a field, seats in the order given; `order` is the server's.
    fn crowd(count: usize) -> (Vec<Token>, Vec<InitiativeRoll>) {
        let tokens: Vec<Token> = (0..count)
            .map(|index| {
                let side = if index < 2 { Side::Hero } else { Side::Enemy };
                token(
                    &format!("c{index}"),
                    side,
                    Cell::new(index as i32 % 8, 1 + index as i32 / 8),
                )
            })
            .collect();
        // The server's order, deliberately not the seat order.
        let mut rolls: Vec<InitiativeRoll> = tokens
            .iter()
            .enumerate()
            .map(|(index, token)| roll(&token.id.0, 3 + (index as u32 * 7) % 17, 1))
            .collect();
        rolls.sort_by_key(|roll| std::cmp::Reverse(roll.total));
        (tokens, rolls)
    }

    fn badge_order(timeline: &Timeline) -> Vec<(String, u32)> {
        timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::InitiativeBadge { token, order, .. } => Some((token.0.clone(), *order)),
                _ => None,
            })
            .collect()
    }

    fn whip_targets(timeline: &Timeline) -> Vec<String> {
        timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::Whip {
                    command:
                        CameraCommand {
                            focus: CameraFocus::Token(id),
                            ..
                        },
                    ..
                } => Some(id.0.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_initiative_runs_banner_dice_roll_call_tracker_pull_back_round_banner_first_turn() {
        let (grid, _) = field();
        let (tokens, rolls) = crowd(3);
        let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
        let labels = names(&timeline);
        let order = [
            "init-start",
            "banner-roll",
            "die",
            "snd-dice-rattle",
            "die-land",
            "snd-dice-land",
            "banner-off",
            "whip",
            "ready",
            "badge",
            "tracker",
            "sting",
            "banner-round",
        ];
        let positions: Vec<usize> = order.iter().map(|name| position(&labels, name)).collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{labels:?}"
        );
        assert_eq!(labels.last(), Some(&"highlight"));
        assert!(
            labels[labels.len() - 3..].contains(&"focus"),
            "ends on the first turn-focus shot"
        );
        // Every die lands before the first stop; the last one is the pull-back whip.
        let last_land = *times_of(&timeline, "die-land").last().unwrap();
        let first_whip = times_of(&timeline, "whip")[0];
        assert!(last_land < first_whip);
        let whips = times_of(&timeline, "whip");
        let tracker = times_of(&timeline, "tracker");
        assert_eq!(tracker.len(), 3);
        assert!(
            tracker.windows(2).all(|pair| pair[0] < pair[1]),
            "fills in order, one stop at a time"
        );
        assert_eq!(whips.len(), 4, "three stops and the pull-back");
    }

    #[test]
    fn the_stops_follow_the_servers_order_and_the_badges_count_up() {
        let (grid, _) = field();
        let (tokens, rolls) = crowd(4);
        let server_order: Vec<String> = rolls.iter().map(|roll| roll.token.0.clone()).collect();
        let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
        assert_eq!(whip_targets(&timeline)[..4], server_order[..]);
        let badges = badge_order(&timeline);
        assert_eq!(
            badges.iter().map(|(_, order)| *order).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            badges.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
            server_order
        );
        // The dice are rolled in seat order, which is not the turn order.
        let dice: Vec<String> = timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::InitiativeDie { token, .. } => Some(token.0.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(dice, vec!["c0", "c1", "c2", "c3"]);
        assert_ne!(dice, server_order);
    }

    #[test]
    fn a_tie_is_shown_in_the_order_the_server_resolved_it_whichever_way_that_went() {
        let (grid, _) = field();
        let (tokens, _) = crowd(3);
        for tie_winner in ["c1", "c2"] {
            let loser = if tie_winner == "c1" { "c2" } else { "c1" };
            let rolls = vec![
                roll("c0", 19, 1),
                roll(tie_winner, 12, 2),
                roll(loser, 12, 2),
            ];
            assert_eq!(rolls[1].total, rolls[2].total, "a genuine tie");
            let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
            assert_eq!(whip_targets(&timeline)[..3], ["c0", tie_winner, loser]);
            let badges = badge_order(&timeline);
            assert_eq!(badges[1], (tie_winner.to_owned(), 2));
            assert_eq!(badges[2], (loser.to_owned(), 3));
        }
    }

    #[test]
    fn the_roll_call_never_runs_past_the_cap_however_many_fight() {
        let (grid, _) = field();
        for count in [2usize, 3, 6, 7, 10, 14] {
            let (tokens, rolls) = crowd(count);
            let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
            let first = times_of(&timeline, "whip")[0];
            let pull_back = *times_of(&timeline, "whip").last().unwrap();
            assert!(
                pull_back - first <= MAX_ROLL_CALL,
                "{count} combatants: roll-call {:?}",
                pull_back - first
            );
            // About a second a combatant while that fits.
            if count <= 6 {
                assert!(pull_back - first >= STOP * count as u32 - Duration::from_millis(1));
            }
            assert_eq!(
                badge_order(&timeline).len(),
                count,
                "everyone gets an order badge"
            );
            assert_eq!(times_of(&timeline, "tracker").len(), count);
        }
    }

    #[test]
    fn beyond_six_the_remaining_enemies_are_swept_in_one_stop() {
        let (grid, _) = field();
        let (tokens, rolls) = crowd(9);
        let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
        let whips = times_of(&timeline, "whip");
        assert_eq!(
            whips.len(),
            MAX_INDIVIDUAL_STOPS + 2,
            "six stops, one sweep, the pull-back"
        );
        let sweep_badges: Vec<Duration> = timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::InitiativeBadge { order, .. } if *order as usize > MAX_INDIVIDUAL_STOPS => {
                    Some(timed.at)
                }
                _ => None,
            })
            .collect();
        assert_eq!(sweep_badges.len(), 3);
        assert!(
            sweep_badges.windows(2).all(|pair| pair[0] == pair[1]),
            "all at once"
        );
        assert_eq!(times_of(&timeline, "sting").len(), MAX_INDIVIDUAL_STOPS + 1);
    }

    #[test]
    fn the_stings_rise_and_the_last_one_is_the_heavy_hit() {
        let (grid, _) = field();
        let (tokens, rolls) = crowd(4);
        let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
        let stings: Vec<(u32, bool)> = timeline
            .steps
            .iter()
            .filter_map(|timed| match &timed.step {
                Step::Sting { index, last, .. } => Some((*index, *last)),
                _ => None,
            })
            .collect();
        assert_eq!(stings, vec![(0, false), (1, false), (2, false), (3, true)]);
    }

    #[test]
    fn each_kind_of_combatant_gets_its_own_ready_flourish() {
        let (grid, _) = field();
        let (mut tokens, rolls) = crowd(3);
        tokens[1].attack = AttackStyle::Ranged;
        let timeline = plan(&grid, &tokens, &Beat::Initiative { rolls }).unwrap();
        let mut styles = std::collections::BTreeMap::new();
        for timed in &timeline.steps {
            if let Step::Animate {
                token,
                anim: Anim::Ready { style },
            } = &timed.step
            {
                styles.insert(token.0.clone(), *style);
            }
        }
        assert_eq!(styles["c0"], ReadyStyle::Brandish);
        assert_eq!(styles["c1"], ReadyStyle::Nock);
        assert_eq!(styles["c2"], ReadyStyle::Snarl);
    }

    #[test]
    fn under_reduced_motion_the_dice_appear_landed_and_the_sequence_is_shorter() {
        let (grid, _) = field();
        let (tokens, rolls) = crowd(3);
        let beat = Beat::Initiative { rolls };
        let calm = plan_with(&grid, &tokens, &beat, true).unwrap();
        let lively = plan_with(&grid, &tokens, &beat, false).unwrap();
        let labels = names(&calm);
        assert!(!labels.contains(&"snd-dice-rattle") && !labels.contains(&"die-land"));
        assert!(calm.duration < lively.duration);
        assert_eq!(
            whip_targets(&calm),
            whip_targets(&lively),
            "same order, same information"
        );
        assert_eq!(badge_order(&calm), badge_order(&lively));
    }

    #[test]
    fn an_initiative_for_an_unknown_token_is_a_planning_error_and_an_empty_one_is_empty() {
        let (grid, _) = field();
        let (tokens, _) = crowd(3);
        assert_eq!(
            plan(
                &grid,
                &tokens,
                &Beat::Initiative {
                    rolls: vec![roll("ghost", 1, 0)]
                }
            ),
            Err(PlanError::UnknownToken(TokenId::new("ghost")))
        );
        let empty = plan(&grid, &tokens, &Beat::Initiative { rolls: vec![] }).unwrap();
        assert!(empty.steps.is_empty());
    }
}
