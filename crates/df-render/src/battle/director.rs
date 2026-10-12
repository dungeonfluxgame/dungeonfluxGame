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
        AttackResult, Beat, CameraCommand, CameraFocus, FloatTone, Highlight, HighlightKind, Side,
        Token, TokenId,
    },
};

const FOCUS_PAN: Duration = Duration::from_millis(700);
const FOCUS_HOLD: Duration = Duration::from_millis(250);
const STEP: Duration = Duration::from_millis(300);
const IMPACT_PAN: Duration = Duration::from_millis(380);
const SETTLE_PAN: Duration = Duration::from_millis(900);
const FINISHING_PUSH: Duration = Duration::from_millis(3200);
const FINISHING_HOLD: Duration = Duration::from_millis(3600);
pub const LUNGE: Duration = Duration::from_millis(460);
pub const LUNGE_CONTACT: Duration = Duration::from_millis(230);
pub const RECOIL: Duration = Duration::from_millis(420);
pub const FALL: Duration = Duration::from_millis(900);
pub const HEAL: Duration = Duration::from_millis(1300);
const FALL_DELAY: Duration = Duration::from_millis(380);
const FINISHING_DELAY: Duration = Duration::from_millis(560);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Anim {
    Lunge { toward: TokenId },
    Recoil { from: TokenId },
    Dodge { from: TokenId },
    Fall,
    Heal,
}

impl Anim {
    pub fn duration(&self) -> Duration {
        match self {
            Self::Lunge { .. } => LUNGE,
            Self::Recoil { .. } | Self::Dodge { .. } => RECOIL,
            Self::Fall => FALL,
            Self::Heal => HEAL,
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
    /// Move a token cell by cell; `path` excludes the cell it starts on.
    Walk {
        token: TokenId,
        path: Vec<Cell>,
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

/// Plans the staging of `beat` against the current formation.
pub fn plan(grid: &Grid, tokens: &[Token], beat: &Beat) -> Result<Timeline, PlanError> {
    match beat {
        Beat::TurnStart { actor } => plan_turn(tokens, actor),
        Beat::Attack {
            attacker,
            target,
            result,
            target_hit_points_after,
        } => plan_attack(
            grid,
            tokens,
            attacker,
            target,
            *result,
            *target_hit_points_after,
        ),
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

fn plan_attack(
    grid: &Grid,
    tokens: &[Token],
    attacker_id: &TokenId,
    target_id: &TokenId,
    result: AttackResult,
    hit_points_after: Option<u32>,
) -> Result<Timeline, PlanError> {
    let attacker = find(tokens, attacker_id)?;
    let target = find(tokens, target_id)?;
    let mut builder = Builder { steps: Vec::new() };
    builder.at(Duration::ZERO, Step::SetActive(Some(attacker_id.clone())));
    builder.at(
        Duration::ZERO,
        Step::Camera {
            command: camera(
                ShotName::TurnFocus,
                CameraFocus::Token(attacker_id.clone()),
                1.0,
            ),
            pan: FOCUS_PAN,
        },
    );
    let mut now = FOCUS_PAN + FOCUS_HOLD;

    // Step next to the target when not already beside it.
    let occupied: Vec<Cell> = tokens
        .iter()
        .filter(|token| token.id != *attacker_id)
        .map(|token| token.cell)
        .collect();
    let approach = (attacker.cell.steps_to(target.cell) > 1)
        .then(|| grid.free_cell_beside(target.cell, attacker.cell, &occupied))
        .flatten()
        .and_then(|beside| grid.path(attacker.cell, beside));
    let stepped = approach.as_ref().is_some_and(|path| path.len() > 1);
    if let Some(path) = approach.filter(|path| path.len() > 1) {
        let steps: Vec<Cell> = path[1..].to_vec();
        let walk = STEP * steps.len() as u32;
        builder.at(
            now,
            Step::Walk {
                token: attacker_id.clone(),
                path: steps,
                per_step: STEP,
            },
        );
        now += walk;
    }
    let beside = approach_end(&builder, attacker.cell);

    builder.at(
        now,
        Step::Highlight(Some(Highlight {
            kind: HighlightKind::Attack,
            cells: vec![target.cell],
        })),
    );
    builder.at(
        now,
        Step::Animate {
            token: attacker_id.clone(),
            anim: Anim::Lunge {
                toward: target_id.clone(),
            },
        },
    );
    let contact = now + LUNGE_CONTACT;
    let mut end = now + LUNGE;
    match result {
        AttackResult::Miss => {
            builder.at(
                contact,
                Step::Camera {
                    command: camera(
                        ShotName::Impact,
                        CameraFocus::Between(attacker_id.clone(), target_id.clone()),
                        1.0,
                    ),
                    pan: IMPACT_PAN,
                },
            );
            builder.at(
                contact,
                Step::Animate {
                    token: target_id.clone(),
                    anim: Anim::Dodge {
                        from: attacker_id.clone(),
                    },
                },
            );
            builder.at(
                contact,
                Step::Float {
                    token: target_id.clone(),
                    text: "Miss".to_owned(),
                    tone: FloatTone::Miss,
                },
            );
        }
        AttackResult::Hit {
            damage,
            critical,
            downed,
        } => {
            builder.at(
                contact,
                Step::Camera {
                    command: camera(
                        ShotName::Impact,
                        CameraFocus::Between(attacker_id.clone(), target_id.clone()),
                        1.0,
                    ),
                    pan: IMPACT_PAN,
                },
            );
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
                    token: target_id.clone(),
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
                    token: target_id.clone(),
                    anim: Anim::Recoil {
                        from: attacker_id.clone(),
                    },
                },
            );
            if let Some(current) = hit_points_after {
                builder.at(
                    contact,
                    Step::ShowHitPoints {
                        token: target_id.clone(),
                        current,
                    },
                );
            }
            if downed {
                let fall = contact + FALL_DELAY;
                builder.at(
                    fall,
                    Step::Animate {
                        token: target_id.clone(),
                        anim: Anim::Fall,
                    },
                );
                if target.side == Side::Enemy && attacker.side == Side::Hero {
                    let cue = fall + FINISHING_DELAY;
                    builder.at(
                        cue,
                        Step::FinishingBlow {
                            hero: attacker_id.clone(),
                            enemy: target_id.clone(),
                        },
                    );
                    builder.at(
                        cue,
                        Step::Camera {
                            command: camera(
                                ShotName::Victory,
                                CameraFocus::Token(attacker_id.clone()),
                                0.7,
                            ),
                            pan: FINISHING_PUSH,
                        },
                    );
                    let over = cue + FINISHING_HOLD;
                    builder.at(over, Step::EndFinishingBlow);
                    return Ok(builder.finish(over));
                }
                builder.at(
                    fall,
                    Step::Camera {
                        command: camera(ShotName::Ko, CameraFocus::Token(target_id.clone()), 1.0),
                        pan: SETTLE_PAN,
                    },
                );
                end = fall + FALL;
                return Ok(builder.finish(end));
            }
        }
    }

    // Settle: everyone returns to formation and the camera widens.
    end += Duration::from_millis(220);
    builder.at(end, Step::Highlight(None));
    if stepped {
        let home = grid.path(beside, attacker.cell).unwrap_or_default();
        if home.len() > 1 {
            let steps: Vec<Cell> = home[1..].to_vec();
            let walk = STEP * steps.len() as u32;
            builder.at(
                end,
                Step::Walk {
                    token: attacker_id.clone(),
                    path: steps,
                    per_step: STEP,
                },
            );
            end += walk;
        }
    }
    builder.at(
        end,
        Step::Camera {
            command: camera(ShotName::CombatEst, CameraFocus::Scene, 1.0),
            pan: SETTLE_PAN,
        },
    );
    Ok(builder.finish(end + SETTLE_PAN))
}

/// The cell the attacker ends its approach on: the last cell of a queued walk, else where it was.
fn approach_end(builder: &Builder, start: Cell) -> Cell {
    builder
        .steps
        .iter()
        .rev()
        .find_map(|timed| match &timed.step {
            Step::Walk { path, .. } => path.last().copied(),
            _ => None,
        })
        .unwrap_or(start)
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
                Step::Walk { .. } => "walk",
                Step::Animate { anim, .. } => match anim {
                    Anim::Lunge { .. } => "lunge",
                    Anim::Recoil { .. } => "recoil",
                    Anim::Dodge { .. } => "dodge",
                    Anim::Fall => "fall",
                    Anim::Heal => "heal",
                },
                Step::Shake { .. } => "shake",
                Step::Flash { .. } => "flash",
                Step::Float { .. } => "float",
                Step::ShowHitPoints { .. } => "hp",
                Step::FinishingBlow { .. } => "finishing",
                Step::EndFinishingBlow => "finishing-end",
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
        let walks: Vec<&Step> = timeline
            .steps
            .iter()
            .map(|timed| &timed.step)
            .filter(|step| matches!(step, Step::Walk { .. }))
            .collect();
        assert_eq!(walks.len(), 2, "out and back");
        let Step::Walk { path: out, .. } = walks[0] else {
            unreachable!()
        };
        let Step::Walk { path: back, .. } = walks[1] else {
            unreachable!()
        };
        let beside = *out.last().unwrap();
        assert_eq!(beside.steps_to(Cell::new(4, 1)), 1);
        assert_ne!(beside, Cell::new(4, 1));
        assert_eq!(*back.last().unwrap(), Cell::new(3, 4));
        let names = names(&timeline);
        assert!(!names.contains(&"fall"));
        assert_eq!(names.last(), Some(&"settle-shot"));
    }

    #[test]
    fn an_attacker_already_beside_the_target_does_not_walk() {
        let (grid, mut tokens) = field();
        tokens[0].cell = Cell::new(4, 2);
        let timeline = plan(&grid, &tokens, &attack(AttackResult::Miss)).unwrap();
        assert!(!names(&timeline).contains(&"walk"));
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
}
