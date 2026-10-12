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
    grid::{Cell, Grid},
    model::{
        AttackResult, AttackStyle, Beat, CameraCommand, CameraFocus, FloatTone, Highlight,
        HighlightKind, Side, Token, TokenId,
    },
    sound::SoundKind,
};

const FOCUS_PAN: Duration = Duration::from_millis(650);
const FOCUS_HOLD: Duration = Duration::from_millis(200);
const STEP: Duration = Duration::from_millis(260);
const WALK_START: Duration = Duration::from_millis(150);
const IMPACT_PAN: Duration = Duration::from_millis(380);
const SETTLE_PAN: Duration = Duration::from_millis(900);
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
pub const FALL_LANDS: Duration = Duration::from_millis(560);
/// Extra time the impact shot stays up after a hit that deals damage.
pub const IMPACT_HOLD: Duration = Duration::from_millis(1800);
const FALL_DELAY: Duration = Duration::from_millis(380);
const FINISHING_DELAY: Duration = Duration::from_millis(560);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Anim {
    Lunge { toward: TokenId },
    Recoil { from: TokenId },
    Dodge { from: TokenId },
    Fall,
    Heal,
    /// Lean back drawing a bow toward the target, then snap forward on release.
    Draw { toward: TokenId },
}

impl Anim {
    pub fn duration(&self) -> Duration {
        match self {
            Self::Lunge { .. } => LUNGE,
            Self::Recoil { .. } | Self::Dodge { .. } => RECOIL,
            Self::Fall => FALL,
            Self::Heal => HEAL,
            Self::Draw { .. } => DRAW,
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

fn camera(shot: ShotName, focus: CameraFocus, dolly: f32) -> CameraCommand {
    CameraCommand { shot, focus, dolly }
}

/// Plans the staging of `beat` against the current formation. `tokens` carry their formation
/// (home) cells; the attacker's steps are planned from home and the stage resolves where it
/// actually stands.
pub fn plan(grid: &Grid, tokens: &[Token], beat: &Beat) -> Result<Timeline, PlanError> {
    match beat {
        Beat::TurnStart { actor } => plan_turn(tokens, actor),
        Beat::Approach { attacker, target } => {
            let (attacker, target) = (find(tokens, attacker)?, find(tokens, target)?);
            let mut builder = Builder { steps: Vec::new() };
            let end = approach(&mut builder, grid, tokens, attacker, target, Duration::ZERO);
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
            let beside = standing_cell(grid, tokens, attacker, target);
            builder.at(
                Duration::ZERO,
                Step::MoveTo {
                    token: attacker.id.clone(),
                    cell: beside,
                    per_step: Duration::ZERO,
                },
            );
            let end = strike(
                &mut builder,
                grid,
                attacker,
                target,
                beside,
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
            let ready = approach(&mut builder, grid, tokens, attacker, target, Duration::ZERO);
            let beside = standing_cell(grid, tokens, attacker, target);
            let end = strike(
                &mut builder,
                grid,
                attacker,
                target,
                beside,
                *result,
                *target_hit_points_after,
                ready + FOCUS_HOLD,
            );
            Ok(builder.finish(end))
        }
        Beat::Heal {
            source,
            target,
            amount,
            target_hit_points_after,
        } => plan_heal(tokens, source, target, *amount, *target_hit_points_after),
    }
}

fn plan_turn(tokens: &[Token], actor: &TokenId) -> Result<Timeline, PlanError> {
    let token = find(tokens, actor)?;
    let mut builder = Builder { steps: Vec::new() };
    builder.at(Duration::ZERO, Step::SetActive(Some(actor.clone())));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(ShotName::TurnFocus, CameraFocus::Token(actor.clone()), 1.0),
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

/// The cell the attacker fights from: its own when already beside the target, else the free
/// cell beside the target closest to it. Deterministic, so the approach and the strike agree.
fn fighting_cell(grid: &Grid, tokens: &[Token], attacker: &Token, target: &Token) -> Cell {
    if attacker.cell.steps_to(target.cell) <= 1 {
        return attacker.cell;
    }
    let occupied: Vec<Cell> = tokens
        .iter()
        .filter(|token| token.id != attacker.id)
        .map(|token| token.cell)
        .collect();
    grid.free_cell_beside(target.cell, attacker.cell, &occupied)
        .unwrap_or(attacker.cell)
}

/// Where the attacker stands while it strikes: its own cell for a ranged attacker, otherwise
/// the fighting cell beside the target.
fn standing_cell(grid: &Grid, tokens: &[Token], attacker: &Token, target: &Token) -> Cell {
    match attacker.attack {
        AttackStyle::Ranged => attacker.cell,
        AttackStyle::Melee => fighting_cell(grid, tokens, attacker, target),
    }
}

/// Turn-focus shot on the attacker while it steps next to the target. Returns when it is set.
fn approach(
    builder: &mut Builder,
    grid: &Grid,
    tokens: &[Token],
    attacker: &Token,
    target: &Token,
    base: Duration,
) -> Duration {
    builder.at(base, Step::SetActive(Some(attacker.id.clone())));
    builder.at(
        base,
        Step::Camera {
            command: camera(
                ShotName::TurnFocus,
                CameraFocus::Token(attacker.id.clone()),
                1.0,
            ),
            pan: FOCUS_PAN,
        },
    );
    builder.at(base, Step::Target(Some(target.id.clone())));
    let mut end = base + FOCUS_PAN;
    if attacker.attack == AttackStyle::Ranged {
        return end;
    }
    let beside = fighting_cell(grid, tokens, attacker, target);
    if beside != attacker.cell {
        let steps = grid
            .path(attacker.cell, beside)
            .map_or(0, |path| path.len().saturating_sub(1)) as u32;
        let per_step = walk_pace(steps);
        builder.at(
            base + WALK_START,
            Step::MoveTo {
                token: attacker.id.clone(),
                cell: beside,
                per_step,
            },
        );
        end = end.max(base + WALK_START + per_step * steps);
    }
    end
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
/// `beside` is where a melee attacker stands to strike; it walks back from there afterwards.
#[allow(clippy::too_many_arguments)]
fn strike(
    builder: &mut Builder,
    grid: &Grid,
    attacker: &Token,
    target: &Token,
    beside: Cell,
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
    let impact_camera = Step::Camera {
        command: camera(
            ShotName::Impact,
            CameraFocus::Between(attacker.id.clone(), target.id.clone()),
            1.0,
        ),
        pan: IMPACT_PAN,
    };
    match result {
        AttackResult::Miss => {
            builder.at(contact, impact_camera);
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
            builder.at(contact, impact_camera);
            builder.at(
                contact,
                Step::Shake {
                    amplitude_px: if critical { 14.0 } else { 8.0 },
                    duration: Duration::from_millis(200),
                },
            );
            builder.at(
                contact,
                Step::Flash {
                    tone: if critical {
                        FlashTone::Critical
                    } else {
                        FlashTone::Impact
                    },
                    strength: if critical { 0.9 } else { 0.55 },
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
                    builder.at(
                        cue,
                        Step::Camera {
                            command: camera(
                                ShotName::Victory,
                                CameraFocus::Token(attacker.id.clone()),
                                0.7,
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
                        command: camera(ShotName::Ko, CameraFocus::Token(target.id.clone()), 1.0),
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
    let home = attacker.cell;
    let back_steps = if home == beside {
        0
    } else {
        grid.path(beside, home)
            .map_or(0, |path| path.len().saturating_sub(1)) as u32
    };
    let back_pace = walk_pace(back_steps);
    builder.at(
        end,
        Step::MoveTo {
            token: attacker.id.clone(),
            cell: home,
            per_step: back_pace,
        },
    );
    builder.at(
        end + Duration::from_millis(200),
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: SETTLE_PAN,
        },
    );
    (end + SETTLE_PAN + Duration::from_millis(200)).max(end + back_pace * back_steps)
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
                CameraFocus::Token(source_id.clone()),
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
                },
                Step::Shake { .. } => "shake",
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
                },
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
            "walk",
            "lunge",
            "impact-shot",
            "shake",
            "flash",
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
    fn the_attacker_walks_to_a_free_cell_beside_the_target_and_back() {
        let (grid, tokens) = field();
        let timeline = plan(
            &grid,
            &tokens,
            &attack(AttackResult::Hit {
                damage: 3,
                critical: false,
                downed: false,
            }),
        )
        .unwrap();
        let moves = moves(&timeline);
        assert_eq!(moves.len(), 2, "out and back");
        let beside = moves[0].0;
        assert_eq!(beside.steps_to(Cell::new(4, 1)), 1);
        assert_ne!(beside, Cell::new(4, 1));
        assert_eq!(moves[1].0, Cell::new(3, 4));
        let names = names(&timeline);
        assert!(!names.contains(&"fall"));
        assert_eq!(names.last(), Some(&"settle-shot"));
    }

    #[test]
    fn an_attacker_already_beside_the_target_never_leaves_its_cell() {
        let (grid, mut tokens) = field();
        tokens[0].cell = Cell::new(4, 2);
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        assert!(
            moves(&timeline)
                .iter()
                .all(|(cell, _)| *cell == Cell::new(4, 2))
        );
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
        assert_eq!(first, vec!["active", "focus", "target", "walk"]);
        assert!(!first.contains(&"lunge"));
        let second = names(&strike_only);
        assert!(position(&second, "lunge") < position(&second, "impact-shot"));
        assert!(!second.contains(&"focus"));
        // The strike snaps the attacker beside the target first, in case the approach was skipped.
        assert_eq!(moves(&strike_only)[0].1, Duration::ZERO);
        assert!(approach_only.duration < Duration::from_millis(1200));
    }

    #[test]
    fn a_miss_dodges_without_shake_flash_or_fall() {
        let (grid, tokens) = field();
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        let names = names(&timeline);
        assert!(names.contains(&"dodge") && names.contains(&"float"));
        for absent in ["shake", "flash", "recoil", "fall", "finishing"] {
            assert!(!names.contains(&absent), "{absent} in {names:?}");
        }
    }

    #[test]
    fn a_critical_hit_shakes_and_flashes_harder_than_an_ordinary_one() {
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
                    Step::Flash { strength, .. } => Some(strength),
                    _ => None,
                })
                .unwrap()
        };
        assert!(strength(true) > strength(false));
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
        assert!(!labels.contains(&"walk") || moves(&timeline).iter().all(|(cell, _)| *cell == Cell::new(3, 4)));
        let draw = times_of(&timeline, "draw")[0];
        let release = times_of(&timeline, "snd-bow-release")[0];
        let flight = times_of(&timeline, "arrow")[0];
        let impact = times_of(&timeline, "snd-arrow-impact")[0];
        let number = times_of(&timeline, "float")[0];
        assert!(draw < release && release == flight && flight < impact);
        assert_eq!(impact - release, FLIGHT, "the blow lands when the arrow arrives");
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
        assert!(moves(&timeline).iter().all(|(cell, _)| *cell == Cell::new(0, 5)));
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
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{labels:?}");
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
        assert!(hit >= Duration::from_millis(2400), "the number lives 1.4 s and is read: {hit:?}");
    }

    #[test]
    fn a_long_walk_is_quickened_so_the_fight_does_not_stall() {
        assert_eq!(walk_pace(2), STEP);
        let steps = 12;
        let pace = walk_pace(steps);
        assert!(pace < STEP && pace >= MIN_STEP);
        assert!(pace * steps <= MAX_APPROACH + Duration::from_millis(12));
    }

    #[test]
    fn the_approach_goes_around_an_obstacle_not_through_it() {
        let (grid, tokens) = field();
        let wall: Vec<Cell> = (1..7).map(|col| Cell::new(col, 3)).collect();
        let blocked = Grid::open_except([0.0, 0.0], 1.5, 8, 6, &wall).unwrap();
        let timeline = plan(&blocked, &tokens, &attack(AttackResult::Miss)).unwrap();
        let beside = moves(&timeline)[0].0;
        let path = blocked.path(Cell::new(3, 4), beside).unwrap();
        assert!(path.iter().all(|cell| blocked.is_walkable(*cell)));
        assert!(path.len() > 4, "detours around the wall: {path:?}");
        assert!(blocked.is_walkable(beside));
        let _ = grid;
    }
}
