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
    /// Outcome sequence that last put this member in `cell`, or zero when the sequence is unknown.
    pub position_sequence: u64,
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
    /// The weapon the server says was used (its id, as sent); empty when it did not say.
    pub weapon: String,
    /// A graze: the blow missed but its mastery still dealt `damage`.
    pub grazed: bool,
    /// The damage type the server's rules name ("slashing"); empty when it did not say.
    pub damage_type: String,
}

impl Outcome {
    /// The attack's name for the result shown at the target: "Graze" for a graze, else the
    /// weapon named by the server; `None` when the view carries neither.
    pub fn attack_label(&self) -> Option<String> {
        if self.grazed && !self.hit {
            return Some("Graze".to_owned());
        }
        let name = super::float_text::attack_name(&self.weapon);
        (!name.is_empty()).then_some(name)
    }
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
    /// Latest move or attack sequence carried by the server snapshot.
    pub latest_sequence: u64,
    /// Highest outcome sequence whose result the display has shown.
    pub presented_sequence: u64,
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

fn sequence_was_presented(sequence: u64, presented_sequence: u64) -> bool {
    sequence != 0 && sequence <= presented_sequence
}

fn active_was_presented(snapshot: &CombatSnapshot) -> bool {
    let Some(active) = snapshot.active.as_deref() else {
        return false;
    };
    if snapshot.latest_sequence == 0 || snapshot.latest_sequence > snapshot.presented_sequence {
        return false;
    }
    let Some(member) = snapshot.members.iter().find(|member| member.id == active) else {
        return false;
    };
    if snapshot.latest_sequence < snapshot.presented_sequence {
        return sequence_was_presented(member.position_sequence, snapshot.presented_sequence);
    }
    if let Some(outcome) = snapshot.outcomes.last() {
        return outcome.actor == active;
    }
    member.position_sequence != 0 && member.position_sequence < snapshot.presented_sequence
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
        let roster_changed = roster != self.roster;
        let stale_active = self.started && !roster_changed && active_was_presented(snapshot);
        let tokens: Vec<Token> = snapshot
            .members
            .iter()
            .filter_map(|member| {
                let seat = match member.side {
                    Side::Hero => hero_cells.next(),
                    Side::Enemy => enemy_cells.next(),
                };
                let mut token = token_for(member, member.cell.or(seat)?);
                if self.started
                    && !roster_changed
                    && sequence_was_presented(member.position_sequence, snapshot.presented_sequence)
                    && let Some(current) = stage.tokens().find(|current| current.id == token.id)
                {
                    // A stale view may still carry a cell from a position already walked by its
                    // cue. Keep the stage's position so set_tokens cannot replay that movement.
                    token.cell = current.cell;
                }
                Some(token)
            })
            .collect();
        let mut report = FeedReport::default();

        if !self.started || roster_changed {
            // A new fight (or a changed roster): set the scene without replaying anything.
            stage.set_tokens_at_sequence(tokens, snapshot.presented_sequence);
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
            let fresh_start = self.seen_outcomes;
            let fresh: Vec<&Outcome> = snapshot.outcomes.iter().skip(fresh_start).collect();
            let beat_rounds =
                rounds_for_outcomes(&snapshot.outcomes, &snapshot.initiative, &snapshot.members);
            self.seen_outcomes = snapshot.outcomes.len();
            // What a member's hit points are compared with to notice a heal. A cue-driven display
            // is given hit points as of the beat it has reached, so a fresh outcome's figure can
            // be lower than what is shown without anyone having been healed: there only the last
            // value this feed was given counts. (Comparing with the fresh outcome staged a heal
            // back to the old number whenever the display was behind: "heals, then resets".)
            let cue_driven = self.mode == FeedMode::HoldForCues;
            let heal_basis = |member: &Member| {
                let shown = self.last_hit_points.get(&member.id).copied();
                if cue_driven {
                    return shown;
                }
                fresh
                    .iter()
                    .rev()
                    .find(|outcome| outcome.target == member.id)
                    .map(|outcome| outcome.target_hit_points_after)
                    .or(shown)
            };
            let healed_members: Vec<(&Member, u32)> = snapshot
                .members
                .iter()
                .filter_map(|member| {
                    heal_basis(member)
                        .filter(|before| member.hit_points > *before)
                        .map(|before| (member, before))
                })
                .collect();
            let mut targets: Vec<TokenId> = fresh
                .iter()
                .map(|outcome| TokenId::new(outcome.target.clone()))
                .collect();
            targets.extend(
                healed_members
                    .iter()
                    .map(|(member, _)| TokenId::new(member.id.clone())),
            );
            stage.hold_hit_points(&targets);
            stage.set_tokens_at_sequence(tokens, snapshot.presented_sequence);
            if self.mode == FeedMode::StageOutcomes {
                for (outcome, round) in fresh.iter().zip(beat_rounds.iter().skip(fresh_start)) {
                    if let Some(label) = outcome.attack_label() {
                        stage.set_attack_label(
                            &TokenId::new(outcome.actor.clone()),
                            &label,
                            &outcome.damage_type,
                        );
                    }
                    match stage.stage_in_round(beat_for(outcome), *round) {
                        Ok(()) => report.staged += 1,
                        Err(_) => report.dropped += 1,
                    }
                }
            }
            for member in &snapshot.members {
                let healed = healed_members
                    .iter()
                    .find(|(healed, _)| healed.id == member.id);
                if let Some((_, heal_from)) = healed {
                    let amount = member.hit_points - heal_from;
                    let beat = Beat::Heal {
                        source: TokenId::new(member.id.clone()),
                        target: TokenId::new(member.id.clone()),
                        amount,
                        target_hit_points_after: Some(member.hit_points),
                    };
                    match stage.stage_in_round(beat, snapshot.round) {
                        Ok(()) => report.staged += 1,
                        Err(_) => report.dropped += 1,
                    }
                }
                self.last_hit_points
                    .insert(member.id.clone(), member.hit_points);
            }
            if report.staged == 0
                && self.mode == FeedMode::StageOutcomes
                && fresh.is_empty()
                && snapshot.active != self.last_active
                && !stale_active
                && !sequence_was_presented(snapshot.latest_sequence, snapshot.presented_sequence)
                && let Some(active) = &snapshot.active
            {
                let beat = Beat::TurnStart {
                    actor: TokenId::new(active.clone()),
                };
                if stage.stage_in_round(beat, snapshot.round).is_err() {
                    report.dropped += 1;
                }
            }
            if !stale_active {
                self.last_active = snapshot.active.clone();
            }
        }
        stage.set_round(snapshot.round);
        if !stale_active {
            stage.set_active(snapshot.active.clone().map(TokenId::new));
        }
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

fn rounds_for_outcomes(
    outcomes: &[Outcome],
    initiative: &[InitiativeEntry],
    members: &[Member],
) -> Vec<u32> {
    let order: Vec<&str> = if initiative.is_empty() {
        members.iter().map(|member| member.id.as_str()).collect()
    } else {
        initiative.iter().map(|entry| entry.id.as_str()).collect()
    };
    let mut round = 1;
    let mut previous_index = None;
    outcomes
        .iter()
        .map(|outcome| {
            let index = order.iter().position(|id| *id == outcome.actor);
            if let (Some(previous), Some(current)) = (previous_index, index)
                && current < previous
            {
                round += 1;
            }
            if index.is_some() {
                previous_index = index;
            }
            round
        })
        .collect()
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
            position_sequence: 0,
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
            latest_sequence: 0,
            presented_sequence: 0,
            outcomes,
            initiative: vec![],
            begun_live: false,
        }
    }

    fn stage() -> Stage {
        Stage::new(load_embedded("salt-quay").unwrap().unwrap())
    }

    #[test]
    fn an_equal_presented_sequence_accepts_the_server_next_actor() {
        let mut stage = stage();
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(&mut stage, &snapshot(Vec::new(), [12, 12, 12]))
            .unwrap();

        let mut completed = snapshot(vec![strike(true, 1, 11, false)], [12, 12, 11]);
        completed.latest_sequence = 6;
        completed.presented_sequence = 6;
        completed.active = Some("b".to_owned());
        completed.members[1].position_sequence = 6;
        feed.apply(&mut stage, &completed).unwrap();

        assert_eq!(
            stage.hud().active.as_ref().map(|active| active.0.as_str()),
            Some("b"),
            "the view at the presented sequence already names the next actor"
        );
    }

    #[test]
    fn a_stale_view_queued_during_a_strike_cannot_restore_the_old_hit_points() {
        use crate::battle::{AttackResult, Beat, TokenId};
        use std::time::Duration;

        let mut stage = stage();
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(&mut stage, &snapshot(Vec::new(), [12, 12, 11]))
            .unwrap();
        stage
            .stage(Beat::Strike {
                attacker: TokenId::new("a"),
                target: TokenId::new("e"),
                result: AttackResult::Hit {
                    damage: 9,
                    critical: false,
                    downed: false,
                },
                target_hit_points_after: Some(2),
            })
            .unwrap();

        let stale = snapshot(Vec::new(), [12, 12, 11]);
        stage.protect_hit_points_through(1);
        feed.apply(&mut stage, &stale).unwrap();
        let mut samples = vec![
            stage
                .frame()
                .figures
                .iter()
                .find(|figure| figure.id.0 == "e")
                .and_then(|figure| figure.hit_points)
                .unwrap()
                .current,
        ];
        for _ in 0..150 {
            stage.update(Duration::from_millis(100));
            samples.push(
                stage
                    .frame()
                    .figures
                    .iter()
                    .find(|figure| figure.id.0 == "e")
                    .and_then(|figure| figure.hit_points)
                    .unwrap()
                    .current,
            );
            if !stage.is_busy() {
                break;
            }
        }

        assert!(samples.contains(&2), "the strike must land: {samples:?}");
        assert_eq!(samples.last(), Some(&2), "hit points rewound: {samples:?}");
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
            weapon: String::new(),
            grazed: false,
            damage_type: String::new(),
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
    fn a_downing_outcome_ends_with_the_enemy_fallen_without_a_finishing_card() {
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [12, 12, 5]))
            .unwrap();
        feed.apply(
            &mut stage,
            &snapshot(vec![strike(true, 6, 0, true)], [12, 12, 0]),
        )
        .unwrap();
        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
            assert!(stage.hud().finishing_blow.is_none());
        }
        let frame = stage.frame();
        let enemy = frame.figures.iter().find(|f| f.id.0 == "e").unwrap();
        assert!(enemy.downed && enemy.slump > 0.99);
    }

    #[test]
    fn a_healed_hit_point_bar_waits_until_the_heal_beat_is_presented() {
        fn shown_hp(stage: &Stage) -> Option<u32> {
            stage
                .frame()
                .figures
                .iter()
                .find(|figure| figure.id.0 == "a")
                .and_then(|figure| figure.hit_points)
                .map(|hp| hp.current)
        }
        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [6, 12, 12]))
            .unwrap();

        let report = feed
            .apply(&mut stage, &snapshot(vec![], [10, 12, 12]))
            .unwrap();

        assert_eq!(report.staged, 1);
        assert_eq!(shown_hp(&stage), Some(6));

        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
        }
        assert_eq!(shown_hp(&stage), Some(10));
    }

    #[test]
    fn a_heal_after_a_fresh_attack_is_staged_after_the_attack() {
        fn shown_hp(stage: &Stage) -> Option<u32> {
            stage
                .frame()
                .figures
                .iter()
                .find(|figure| figure.id.0 == "a")
                .and_then(|figure| figure.hit_points)
                .map(|hp| hp.current)
        }

        let mut stage = stage();
        let mut feed = CombatFeed::default();
        feed.apply(&mut stage, &snapshot(vec![], [8, 12, 12]))
            .unwrap();
        let mut attack = strike(true, 4, 4, false);
        attack.actor = "e".to_owned();
        attack.target = "a".to_owned();

        let report = feed
            .apply(&mut stage, &snapshot(vec![attack], [10, 12, 12]))
            .unwrap();

        assert_eq!(report.staged, 2);
        assert_eq!(shown_hp(&stage), Some(8));
        let mut saw_attack_hp = false;
        for _ in 0..400 {
            stage.update(Duration::from_millis(50));
            saw_attack_hp |= shown_hp(&stage) == Some(4);
        }
        assert!(saw_attack_hp);
        assert_eq!(shown_hp(&stage), Some(10));
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
    fn cue_owned_feed_does_not_replay_the_server_turn_during_enemy_beats() {
        let mut stage = stage();
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(&mut stage, &snapshot(vec![], [12, 12, 12]))
            .unwrap();

        let mut redraw = snapshot(vec![], [12, 12, 12]);
        redraw.active = Some("b".to_owned());
        feed.apply(&mut stage, &redraw).unwrap();

        assert!(!stage.is_busy());
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
mod round_tests {
    use super::*;
    use crate::battle::scene_file::load_embedded;
    use std::time::Duration;

    fn snapshot(outcomes: Vec<Outcome>, hp: [u32; 3]) -> CombatSnapshot {
        let member = |id: &str, side| Member {
            id: id.to_owned(),
            name: id.to_uppercase(),
            side,
            hit_points: hp[match id {
                "a" => 0,
                "b" => 1,
                _ => 2,
            }],
            maximum_hit_points: 12,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: AttackStyle::Melee,
            voice: None,
            cell: None,
            position_sequence: 0,
            statuses: Vec::new(),
        };
        CombatSnapshot {
            members: vec![
                member("a", Side::Hero),
                member("b", Side::Hero),
                member("e", Side::Enemy),
            ],
            active: Some("a".to_owned()),
            round: 1,
            latest_sequence: 0,
            presented_sequence: 0,
            outcomes,
            initiative: Vec::new(),
            begun_live: false,
        }
    }

    fn strike() -> Outcome {
        Outcome {
            actor: "a".to_owned(),
            target: "e".to_owned(),
            hit: true,
            critical: false,
            damage: 1,
            target_hit_points_after: 11,
            downed: false,
            weapon: String::new(),
            grazed: false,
            damage_type: String::new(),
        }
    }

    #[test]
    fn outcome_rounds_advance_only_when_initiative_order_wraps() {
        let mut outcomes = vec![strike(), strike(), strike(), strike()];
        for (outcome, actor) in outcomes.iter_mut().zip(["e", "e", "a", "e"]) {
            outcome.actor = actor.to_owned();
        }
        let initiative = vec![
            InitiativeEntry {
                id: "a".to_owned(),
                die: 12,
                modifier: 0,
                total: 12,
            },
            InitiativeEntry {
                id: "b".to_owned(),
                die: 10,
                modifier: 0,
                total: 10,
            },
            InitiativeEntry {
                id: "e".to_owned(),
                die: 8,
                modifier: 0,
                total: 8,
            },
        ];
        let members = Vec::new();

        assert_eq!(
            rounds_for_outcomes(&outcomes, &initiative, &members),
            [1, 1, 2, 2]
        );
    }
    #[test]
    fn a_queued_round_one_enemy_beat_keeps_round_one_under_a_round_two_view() {
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        let mut feed = CombatFeed::default();
        let mut opening = snapshot(vec![], [12, 12, 12]);
        opening.active = Some("e".to_owned());
        opening.initiative = vec![
            InitiativeEntry {
                id: "e".to_owned(),
                die: 10,
                modifier: 0,
                total: 10,
            },
            InitiativeEntry {
                id: "a".to_owned(),
                die: 9,
                modifier: 0,
                total: 9,
            },
            InitiativeEntry {
                id: "b".to_owned(),
                die: 8,
                modifier: 0,
                total: 8,
            },
        ];
        feed.apply(&mut stage, &opening).unwrap();

        let mut outcome = strike();
        outcome.actor = "e".to_owned();
        let mut next = snapshot(vec![outcome], [12, 12, 11]);
        next.active = Some("a".to_owned());
        next.round = 2;
        next.initiative = opening.initiative;
        feed.apply(&mut stage, &next).unwrap();
        stage.update(Duration::from_millis(1));

        assert_eq!(stage.hud().round, 1);
        assert_eq!(
            stage.hud().active.as_ref().map(|id| id.0.as_str()),
            Some("e")
        );

        for _ in 0..400 {
            if !stage.is_busy() {
                break;
            }
            stage.update(Duration::from_millis(50));
        }

        assert!(!stage.is_busy());
        assert_eq!(stage.hud().round, 2);
        assert_eq!(
            stage.hud().active.as_ref().map(|id| id.0.as_str()),
            Some("a")
        );
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
            position_sequence: 0,
            statuses: Vec::new(),
        };
        let snapshot = |hp, outcomes| CombatSnapshot {
            members: vec![member("a", Side::Hero, 12), member("e", Side::Enemy, hp)],
            active: Some("a".to_owned()),
            round: 1,
            latest_sequence: 0,
            presented_sequence: 0,
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
            weapon: String::new(),
            grazed: false,
            damage_type: String::new(),
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

    #[test]
    fn in_cue_mode_a_display_behind_the_server_stages_no_heal_and_a_real_heal_still_shows() {
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
            maximum_hit_points: 13,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: AttackStyle::Melee,
            voice: None,
            cell: None,
            position_sequence: 0,
            statuses: Vec::new(),
        };
        let bite = |after| Outcome {
            actor: "e".to_owned(),
            target: "a".to_owned(),
            hit: true,
            critical: false,
            damage: 6,
            target_hit_points_after: after,
            downed: false,
            weapon: String::new(),
            grazed: false,
            damage_type: String::new(),
        };
        // The hero's hit points as the display may show them at each moment, with every outcome
        // the server has resolved so far.
        let snapshot = |shown, outcomes| CombatSnapshot {
            members: vec![member("a", Side::Hero, shown), member("e", Side::Enemy, 9)],
            active: Some("a".to_owned()),
            round: 2,
            latest_sequence: 0,
            presented_sequence: 0,
            outcomes,
            initiative: vec![],
            begun_live: false,
        };
        feed.apply(&mut stage, &snapshot(13, vec![])).unwrap();
        // Two bites arrive together (13 -> 7 -> 1) while the display has presented neither: it
        // is still given 13. That is not a heal from 1 to 13.
        let both = || vec![bite(7), bite(1)];
        assert_eq!(
            feed.apply(&mut stage, &snapshot(13, both()))
                .unwrap()
                .staged,
            0
        );
        // The display reaches the first bite, then the second: still no heal.
        assert_eq!(
            feed.apply(&mut stage, &snapshot(7, both())).unwrap().staged,
            0
        );
        assert_eq!(
            feed.apply(&mut stage, &snapshot(1, both())).unwrap().staged,
            0
        );
        // Second Wind: the shown figure really rises, and that is staged as one heal.
        assert_eq!(
            feed.apply(&mut stage, &snapshot(8, both())).unwrap().staged,
            1
        );
    }
}

#[cfg(test)]
mod presented_move_tests {
    use super::*;
    use crate::battle::{AttackResult, Beat, Cell, TokenId, load_embedded};
    use std::time::Duration;

    fn member(id: &str, side: Side, cell: Cell, position_sequence: u64) -> Member {
        Member {
            id: id.to_owned(),
            name: id.to_owned(),
            side,
            hit_points: if side == Side::Hero { 13 } else { 9 },
            maximum_hit_points: 13,
            height_m: 1.5,
            figure: None,
            portrait: None,
            attack: AttackStyle::Melee,
            voice: None,
            cell: Some(cell),
            position_sequence,
            statuses: Vec::new(),
        }
    }

    fn snapshot(
        cells: [Cell; 4],
        positions: [u64; 4],
        latest_sequence: u64,
        presented_sequence: u64,
        active: &str,
    ) -> CombatSnapshot {
        CombatSnapshot {
            members: vec![
                member("Brakka", Side::Hero, cells[0], positions[0]),
                member("Weasel 1", Side::Enemy, cells[1], positions[1]),
                member("Weasel 2", Side::Enemy, cells[2], positions[2]),
                member("Weasel 3", Side::Enemy, cells[3], positions[3]),
            ],
            active: Some(active.to_owned()),
            round: 1,
            latest_sequence,
            presented_sequence,
            outcomes: Vec::new(),
            initiative: Vec::new(),
            begun_live: false,
        }
    }

    fn remember_heading(stage: &Stage, headings: &mut Vec<String>, previous: &mut Option<String>) {
        let Some(active) = stage.hud().active.as_ref() else {
            return;
        };
        if previous.as_deref() != Some(active.0.as_str()) {
            *previous = Some(active.0.clone());
            headings.push(active.0.clone());
        }
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display ordering only.
    #[test]
    fn live_recorded_round_one_movers_are_not_replayed_after_their_attacks() {
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        stage.set_feel_enabled(false);
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        let mut cells = [
            Cell::new(4, 4),
            Cell::new(1, 1),
            Cell::new(1, 3),
            Cell::new(1, 5),
        ];
        let mut positions = [0; 4];
        feed.apply(&mut stage, &snapshot(cells, positions, 0, 0, "Brakka"))
            .unwrap();
        stage.skip_to_idle();

        let mut headings = Vec::new();
        let mut previous = None;
        let mut hit_points = 13;
        for (index, (actor, move_sequence, attack_sequence)) in
            [("Weasel 1", 1, 2), ("Weasel 2", 3, 4), ("Weasel 3", 5, 6)]
                .into_iter()
                .enumerate()
        {
            let actor_id = TokenId::new(actor);
            let from = cells[index + 1];
            let to = stage
                .grid()
                .reachable_from(from)
                .into_iter()
                .find(|cell| *cell != from && !cells.contains(cell))
                .unwrap();

            stage.hold_active(true);
            stage.show_turn_now(Some(actor_id.clone()));
            remember_heading(&stage, &mut headings, &mut previous);
            stage
                .stage(Beat::TurnStart {
                    actor: actor_id.clone(),
                })
                .unwrap();
            stage.stage_move(&actor_id, to).unwrap();
            stage
                .stage(Beat::Approach {
                    attacker: actor_id.clone(),
                    target: TokenId::new("Brakka"),
                })
                .unwrap();
            let mut elapsed_ms = 0;
            while stage.is_busy() && elapsed_ms < 20_000 {
                stage.update(Duration::from_millis(50));
                elapsed_ms += 50;
                remember_heading(&stage, &mut headings, &mut previous);
            }
            assert!(!stage.is_busy(), "the move and approach should drain");

            cells[index + 1] = to;
            positions[index + 1] = move_sequence;
            hit_points -= 1;
            let mut stale_cells = cells;
            stale_cells[index + 1] = from;
            let stale_active = ["Weasel 1", "Weasel 1", "Weasel 2"][index];
            stage.hold_active(false);
            feed.apply(
                &mut stage,
                &snapshot(
                    stale_cells,
                    positions,
                    attack_sequence,
                    attack_sequence,
                    stale_active,
                ),
            )
            .unwrap();
            remember_heading(&stage, &mut headings, &mut previous);
            assert_eq!(stage.hud().active.as_ref(), Some(&actor_id));
            assert_eq!(
                stage
                    .tokens()
                    .find(|token| token.id == actor_id)
                    .unwrap()
                    .cell,
                to,
                "a presented move must keep its walked-to cell"
            );
            assert!(
                !stage.is_busy(),
                "a presented move must not be staged again"
            );
            stage
                .stage(Beat::Strike {
                    attacker: actor_id,
                    target: TokenId::new("Brakka"),
                    result: AttackResult::Hit {
                        damage: 1,
                        critical: false,
                        downed: false,
                    },
                    target_hit_points_after: Some(hit_points),
                })
                .unwrap();
            let mut elapsed_after_result_ms = None;
            let mut elapsed_ms = 0;
            while stage.is_busy() && elapsed_ms < 20_000 {
                stage.update(Duration::from_millis(50));
                elapsed_ms += 50;
                remember_heading(&stage, &mut headings, &mut previous);
                let shown_hp = stage
                    .frame()
                    .figures
                    .iter()
                    .find(|figure| figure.id.0 == "Brakka")
                    .and_then(|figure| figure.hit_points)
                    .map(|hp| hp.current);
                if elapsed_after_result_ms.is_none() && shown_hp == Some(hit_points) {
                    elapsed_after_result_ms = Some(0);
                } else if let Some(elapsed) = &mut elapsed_after_result_ms {
                    *elapsed += 50;
                }
            }
            assert!(!stage.is_busy(), "the strike should drain");
            if attack_sequence == 6 {
                assert!(
                    elapsed_after_result_ms.is_some_and(|elapsed| elapsed <= 1_500),
                    "the last strike left the stage busy more than 1.5 seconds after its result"
                );
            }
        }

        feed.apply(&mut stage, &snapshot(cells, positions, 6, 6, "Brakka"))
            .unwrap();
        stage.hold_active(false);
        stage.set_active(Some(TokenId::new("Brakka")));
        let mut hero_heading_ms = 0;
        while headings.last().map(String::as_str) != Some("Brakka") && hero_heading_ms < 1_500 {
            stage.update(Duration::from_millis(50));
            hero_heading_ms += 50;
            remember_heading(&stage, &mut headings, &mut previous);
        }
        assert!(
            hero_heading_ms <= 1_500,
            "the hero heading should follow the last blow promptly"
        );
        assert_eq!(headings, ["Weasel 1", "Weasel 2", "Weasel 3", "Brakka"]);
        assert!(!stage.is_busy());
    }
}
