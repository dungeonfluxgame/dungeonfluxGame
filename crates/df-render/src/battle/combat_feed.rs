//! Turns successive audience-safe combat views into stage input: tokens on the derived formation,
//! and one staged beat per newly reported outcome. It is plain data in, stage calls out, so the
//! browser adapter only has to copy the server's view into [`CombatSnapshot`].
//!
//! Everything here follows the server: a hit, a miss, damage and a downing are copied from the
//! reported outcome; a hit-point rise with no outcome is staged as a heal; the first snapshot of a
//! fight only sets the scene (history is never replayed). Cells come from the derived formation
//! (staging only); server-supplied cells would replace [`CombatSnapshot::members`] positions.

use std::collections::BTreeMap;

use super::{
    formation,
    model::{AttackResult, AttackStyle, Beat, HitPoints, InitiativeRoll, Side, Token, TokenId},
    stage::Stage,
    status::Status,
};

/// Who stages a reported outcome.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FeedMode {
    /// The feed stages each new outcome as a whole attack as soon as it is reported.
    #[default]
    StageOutcomes,
    /// Another owner (the display's cue director) stages the attack when its roll and impact
    /// cues play. The feed only keeps the target's hit-point bar from dropping before then.
    HoldForCues,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub side: Side,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    /// Standing height of the figure in metres (dwarf heroes about 1.35, goblins about 1.0).
    pub height_m: f32,
    pub figure: Option<String>,
    pub portrait: Option<String>,
    /// How the figure attacks on screen. Switch point: the server's weapon replaces the adapter's
    /// figure-based guess.
    pub attack: AttackStyle,
    pub voice: Option<super::sound::VoiceSet>,
    /// The cell the server placed the combatant on. `None` stages it on the layout's spawn
    /// cells (or the derived formation); when the server supplies cells the renderer is unchanged.
    pub cell: Option<super::grid::Cell>,
    /// Conditions and states the server reports, plus any the adapter derives from the server's
    /// own knockout state. Hit-point based states (bloodied) are derived at display time.
    pub statuses: Vec<Status>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub actor: String,
    pub target: String,
    pub hit: bool,
    pub critical: bool,
    pub damage: u32,
    pub target_hit_points_after: u32,
    pub downed: bool,
}

/// One recorded initiative roll as the server reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitiativeEntry {
    pub id: String,
    pub die: u32,
    pub modifier: i32,
    pub total: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CombatSnapshot {
    /// Party members in seat order, then enemies.
    pub members: Vec<Member>,
    pub active: Option<String>,
    pub round: u32,
    pub outcomes: Vec<Outcome>,
    /// The server's recorded initiative rolls, first to act first. Empty when none are carried.
    pub initiative: Vec<InitiativeEntry>,
    /// The display watched this fight begin (it showed a non-combat view just before), rather than
    /// loading into a fight already under way. Only then is the initiative moment staged.
    pub begun_live: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeedReport {
    pub staged: u32,
    /// Outcomes that named a participant the stage does not know, or arrived with a full queue.
    pub dropped: u32,
}

#[derive(Debug, Default)]
pub struct CombatFeed {
    pub mode: FeedMode,
    started: bool,
    roster: Vec<String>,
    seen_outcomes: usize,
    last_active: Option<String>,
    last_hit_points: BTreeMap<String, u32>,
}

fn token_for(member: &Member, cell: super::grid::Cell) -> Token {
    Token {
        id: TokenId::new(member.id.clone()),
        side: member.side,
        name: member.name.clone(),
        cell,
        height_m: member.height_m,
        figure: member.figure.clone(),
        portrait: member.portrait.clone(),
        hit_points: (member.maximum_hit_points > 0).then_some(HitPoints {
            current: member.hit_points,
            maximum: member.maximum_hit_points,
        }),
        statuses: member.statuses.clone(),
        attack: member.attack,
        voice: member.voice,
    }
}

impl CombatFeed {
    pub fn new(mode: FeedMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// Brings `stage` in line with `snapshot`. Returns `None` when the formation has no room for
    /// the roster, in which case the caller keeps the flat view.
    pub fn apply(&mut self, stage: &mut Stage, snapshot: &CombatSnapshot) -> Option<FeedReport> {
        let roster: Vec<String> = snapshot.members.iter().map(|m| m.id.clone()).collect();
        let heroes = snapshot
            .members
            .iter()
            .filter(|m| m.side == Side::Hero)
            .count();
        let enemies = snapshot.members.len() - heroes;
        let seats = match stage.layout() {
            Some(layout) => formation::from_layout(layout, stage.grid(), heroes, enemies),
            None => formation::derive(stage.grid(), heroes, enemies),
        }
        .ok()?;
        let (mut hero_cells, mut enemy_cells) =
            (seats.heroes.into_iter(), seats.enemies.into_iter());
        let tokens: Vec<Token> = snapshot
            .members
            .iter()
            .filter_map(|member| {
                let seat = match member.side {
                    Side::Hero => hero_cells.next(),
                    Side::Enemy => enemy_cells.next(),
                };
                Some(token_for(member, member.cell.or(seat)?))
            })
            .collect();
        let roster_changed = roster != self.roster;
        let mut report = FeedReport::default();

        if !self.started || roster_changed {
            // A new fight (or a changed roster): set the scene without replaying anything.
            stage.set_tokens(tokens);
            stage.establish();
            self.started = true;
            self.roster = roster;
            self.seen_outcomes = snapshot.outcomes.len();
            self.last_active = snapshot.active.clone();
            self.last_hit_points = snapshot
                .members
                .iter()
                .map(|member| (member.id.clone(), member.hit_points))
                .collect();
            let opening = snapshot.begun_live
                && snapshot.outcomes.is_empty()
                && snapshot.round <= 1
                && !snapshot.initiative.is_empty();
            if opening {
                let rolls: Vec<InitiativeRoll> = snapshot
                    .initiative
                    .iter()
                    .map(|entry| InitiativeRoll {
                        token: TokenId::new(entry.id.clone()),
                        die: entry.die,
                        modifier: entry.modifier,
                        total: entry.total,
                    })
                    .collect();
                match stage.stage(Beat::Initiative { rolls }) {
                    Ok(()) => report.staged += 1,
                    Err(_) => report.dropped += 1,
                }
            }
        } else {
            if !snapshot.outcomes.is_empty() || snapshot.round > 1 {
                // Someone already acted: the initiative moment is history.
                stage.skip_initiative();
            }
            let fresh: Vec<&Outcome> = snapshot.outcomes.iter().skip(self.seen_outcomes).collect();
            self.seen_outcomes = snapshot.outcomes.len();
            let targets: Vec<TokenId> = fresh
                .iter()
                .map(|outcome| TokenId::new(outcome.target.clone()))
                .collect();
            stage.hold_hit_points(&targets);
            stage.set_tokens(tokens);
            if self.mode == FeedMode::StageOutcomes {
                for outcome in &fresh {
                    match stage.stage(beat_for(outcome)) {
                        Ok(()) => report.staged += 1,
                        Err(_) => report.dropped += 1,
                    }
                }
            }
            for member in &snapshot.members {
                let before = self.last_hit_points.get(&member.id).copied();
                let healed = before.is_some_and(|before| member.hit_points > before)
                    && !fresh.iter().any(|outcome| outcome.target == member.id);
                if healed {
                    let amount = member.hit_points - before.unwrap_or(0);
                    let beat = Beat::Heal {
                        source: TokenId::new(member.id.clone()),
                        target: TokenId::new(member.id.clone()),
                        amount,
                        target_hit_points_after: Some(member.hit_points),
                    };
                    match stage.stage(beat) {
                        Ok(()) => report.staged += 1,
                        Err(_) => report.dropped += 1,
                    }
                }
                self.last_hit_points
                    .insert(member.id.clone(), member.hit_points);
            }
            if report.staged == 0
                && fresh.is_empty()
                && snapshot.active != self.last_active
                && let Some(active) = &snapshot.active
            {
                let beat = Beat::TurnStart {
                    actor: TokenId::new(active.clone()),
                };
                if stage.stage(beat).is_err() {
                    report.dropped += 1;
                }
            }
            self.last_active = snapshot.active.clone();
        }
        stage.set_round(snapshot.round);
        stage.set_active(snapshot.active.clone().map(TokenId::new));
        let order: Vec<TokenId> = if snapshot.initiative.is_empty() {
            snapshot
                .members
                .iter()
                .map(|m| TokenId::new(m.id.clone()))
                .collect()
        } else {
            snapshot
                .initiative
                .iter()
                .map(|entry| TokenId::new(entry.id.clone()))
                .collect()
        };
        stage.set_initiative(order);
        Some(report)
    }
}

fn beat_for(outcome: &Outcome) -> Beat {
    Beat::Attack {
        attacker: TokenId::new(outcome.actor.clone()),
        target: TokenId::new(outcome.target.clone()),
        result: if outcome.hit || outcome.damage > 0 {
            AttackResult::Hit {
                damage: outcome.damage,
                critical: outcome.critical,
                downed: outcome.downed,
            }
        } else {
            AttackResult::Miss
        },
        target_hit_points_after: Some(outcome.target_hit_points_after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::scene_file::load_embedded;
    use std::time::Duration;

    fn member(id: &str, side: Side, hp: u32) -> Member {
        Member {
            id: id.to_owned(),
            name: id.to_uppercase(),
            side,
            hit_points: hp,
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: AttackStyle::Melee,
            voice: None,
            cell: None,
            statuses: Vec::new(),
        }
    }

    fn snapshot(outcomes: Vec<Outcome>, hp: [u32; 3]) -> CombatSnapshot {
        CombatSnapshot {
            members: vec![
                member("a", Side::Hero, hp[0]),
                member("b", Side::Hero, hp[1]),
                member("e", Side::Enemy, hp[2]),
            ],
            active: Some("a".to_owned()),
            round: 1,
            outcomes,
            initiative: vec![],
            begun_live: false,
        }
    }

    fn stage() -> Stage {
        Stage::new(load_embedded("salt-quay").unwrap().unwrap())
    }

    fn strike(hit: bool, damage: u32, after: u32, downed: bool) -> Outcome {
        Outcome {
            actor: "a".to_owned(),
            target: "e".to_owned(),
            hit,
            critical: false,
            damage,
            target_hit_points_after: after,
            downed,
        }
    }

    #[test]
    fn the_first_snapshot_sets_the_scene_and_does_not_replay_history() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        let history = vec![strike(true, 4, 8, false), strike(false, 0, 8, false)];
        let report = feed
            .apply(&mut stage, &snapshot(history, [12, 12, 8]))
            .unwrap();
        assert_eq!(report, FeedReport::default());
        assert!(!stage.is_busy());
        assert_eq!(stage.tokens().count(), 3);
        assert_eq!(stage.hud().round, 1);
    }

    #[test]
    fn a_new_outcome_is_staged_once_and_not_again_on_a_redraw() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [12, 12, 12]))
            .unwrap();
        let after = snapshot(vec![strike(true, 5, 7, false)], [12, 12, 7]);
        assert_eq!(feed.apply(&mut stage, &after).unwrap().staged, 1);
        assert!(stage.is_busy());
        assert_eq!(feed.apply(&mut stage, &after).unwrap().staged, 0);
    }

    #[test]
    fn a_downing_outcome_ends_with_the_enemy_fallen_and_the_finishing_card_shown_then_cleared() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [12, 12, 5]))
            .unwrap();
        feed.apply(
            &mut stage,
            &snapshot(vec![strike(true, 6, 0, true)], [12, 12, 0]),
        )
        .unwrap();
        let mut saw_card = false;
        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
            saw_card |= stage.hud().finishing_blow.is_some();
        }
        assert!(saw_card && stage.hud().finishing_blow.is_none());
        let frame = stage.frame();
        let enemy = frame.figures.iter().find(|f| f.id.0 == "e").unwrap();
        assert!(enemy.downed && enemy.slump > 0.99);
    }

    #[test]
    fn a_hit_point_rise_without_an_outcome_is_staged_as_a_heal() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [6, 12, 12]))
            .unwrap();
        let report = feed
            .apply(&mut stage, &snapshot(vec![], [10, 12, 12]))
            .unwrap();
        assert_eq!(report.staged, 1);
        assert!(stage.is_busy());
    }

    #[test]
    fn a_turn_change_alone_stages_a_turn_focus_and_an_unknown_participant_is_counted() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [12, 12, 12]))
            .unwrap();
        let mut next = snapshot(vec![], [12, 12, 12]);
        next.active = Some("b".to_owned());
        feed.apply(&mut stage, &next).unwrap();
        assert!(stage.is_busy());

        let mut ghost = strike(true, 1, 3, false);
        ghost.actor = "ghost".to_owned();
        let mut with_ghost = snapshot(vec![ghost], [12, 12, 11]);
        with_ghost.active = Some("b".to_owned());
        let report = feed.apply(&mut stage, &with_ghost).unwrap();
        assert_eq!(report.dropped, 1);
    }

    #[test]
    fn a_changed_roster_resets_without_replay() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(
            &mut stage,
            &snapshot(vec![strike(true, 1, 11, false)], [12, 12, 11]),
        )
        .unwrap();
        let mut smaller = snapshot(vec![], [12, 12, 12]);
        smaller.members.pop();
        let report = feed.apply(&mut stage, &smaller).unwrap();
        assert_eq!(report, FeedReport::default());
        assert_eq!(stage.tokens().count(), 2);
    }
}

#[cfg(test)]
mod hold_tests {
    use super::*;
    use crate::battle::scene_file::load_embedded;

    #[test]
    fn in_cue_mode_a_new_outcome_stages_nothing_but_keeps_the_bar_until_the_cue_strikes() {
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        let mut feed = CombatFeed {
            mode: FeedMode::HoldForCues,
            ..CombatFeed::default()
        };
        let member = |id: &str, side, hp| Member {
            id: id.to_owned(),
            name: id.to_uppercase(),
            side,
            hit_points: hp,
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: AttackStyle::Melee,
            voice: None,
            cell: None,
            statuses: Vec::new(),
        };
        let snapshot = |hp, outcomes| CombatSnapshot {
            members: vec![member("a", Side::Hero, 12), member("e", Side::Enemy, hp)],
            active: Some("a".to_owned()),
            round: 1,
            outcomes,
            initiative: vec![],
            begun_live: false,
        };
        feed.apply(&mut stage, &snapshot(12, vec![])).unwrap();
        let hit = Outcome {
            actor: "a".to_owned(),
            target: "e".to_owned(),
            hit: true,
            critical: false,
            damage: 5,
            target_hit_points_after: 7,
            downed: false,
        };
        let report = feed.apply(&mut stage, &snapshot(7, vec![hit])).unwrap();
        assert_eq!(report.staged, 0);
        assert!(!stage.is_busy());
        let shown = |stage: &Stage| {
            stage
                .frame()
                .figures
                .iter()
                .find(|figure| figure.id.0 == "e")
                .and_then(|figure| figure.hit_points)
                .map(|hp| hp.current)
        };
        assert_eq!(shown(&stage), Some(12));
        stage
            .stage(Beat::Strike {
                attacker: TokenId::new("a"),
                target: TokenId::new("e"),
                result: AttackResult::Hit {
                    damage: 5,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(7),
            })
            .unwrap();
        for _ in 0..40 {
            stage.update(std::time::Duration::from_millis(50));
        }
        assert_eq!(shown(&stage), Some(7));
    }
}
