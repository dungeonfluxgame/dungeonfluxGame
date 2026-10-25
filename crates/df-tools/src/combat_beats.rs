//! Pure rules for presenting a fight's beats on the shared display, kept apart from the
//! browser so they run under native tests.
//!
//! The server numbers every outcome in the order it resolved them (`CombatOutcome.sequence`,
//! its place in the fight's recorded history). The display presents beats in exactly that order
//! and applies a beat's consequences (hit points, the down marker, the position) when that beat
//! plays, never when the newest view arrives. This module holds the decisions behind that:
//! ordering and labels, when a creature is shown down and with which statuses, where a move is
//! interrupted by a reaction, and how a display that has fallen behind catches up.
use df_protocol::common as rpc;

/// A display whose oldest waiting beat is older than this plays the waiting beats shortened. It
/// is the "few seconds" after which catch-up begins; nothing is dropped at this stage.
pub(crate) const SHORTEN_LAG_MS: u64 = 4_000;

/// The outcomes of a view in the order the server resolved them: by sequence, once each.
pub(crate) fn in_resolution_order<'a>(
    outcomes: impl IntoIterator<Item = &'a rpc::CombatOutcome>,
) -> Vec<&'a rpc::CombatOutcome> {
    let mut ordered: Vec<&rpc::CombatOutcome> = outcomes.into_iter().collect();
    ordered.sort_by_key(|outcome| outcome.sequence);
    ordered.dedup_by_key(|outcome| outcome.sequence);
    ordered
}

fn kind_of(outcome: &rpc::CombatOutcome) -> Option<rpc::OutcomeKind> {
    rpc::OutcomeKind::try_from(outcome.kind).ok()
}

/// Whether a blow left its target down: knocked out (unconscious at 1 HP) or at 0 hit points.
pub(crate) fn drops_target(outcome: &rpc::CombatOutcome) -> Option<&'static str> {
    if !outcome.hit && !outcome.grazed {
        return None;
    }
    if outcome.knocked_out {
        Some("falls unconscious")
    } else if outcome.target_hit_points == 0 {
        Some("is down")
    } else {
        None
    }
}

/// For each attack of an ordered beat list, its place in a Multiattack when it is one of
/// several consecutive attacks of one attacker on one target: `(position, count)`, positions
/// from 1. A lone attack, an opportunity attack and every non-attack beat are `None`.
pub(crate) fn multiattack_positions(beats: &[&rpc::CombatOutcome]) -> Vec<Option<(usize, usize)>> {
    let mut positions = vec![None; beats.len()];
    let mut start = 0;
    while start < beats.len() {
        let first = beats[start];
        let attack = kind_of(first) == Some(rpc::OutcomeKind::Attack);
        let mut end = start + 1;
        while attack
            && end < beats.len()
            && kind_of(beats[end]) == Some(rpc::OutcomeKind::Attack)
            && beats[end].actor_name == first.actor_name
            && beats[end].target_name == first.target_name
        {
            end += 1;
        }
        let count = end - start;
        if attack && count > 1 {
            for (offset, slot) in positions[start..end].iter_mut().enumerate() {
                *slot = Some((offset + 1, count));
            }
        }
        start = end;
    }
    positions
}

fn count_word(count: usize) -> String {
    match count {
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        count => count.to_string(),
    }
}

/// The words for a beat, labelled by what it was: an opportunity attack, one of a Multiattack's
/// attacks ("makes two attacks (1 of 2)"), and the target that dropped. Plain attacks keep the
/// plain wording.
pub(crate) fn beat_text(outcome: &rpc::CombatOutcome, group: Option<(usize, usize)>) -> String {
    let target = &outcome.target_name;
    let result = if outcome.hit && outcome.critical {
        format!("critical hit for {}", outcome.damage)
    } else if outcome.hit {
        format!("hit for {}", outcome.damage)
    } else if outcome.grazed {
        format!("graze for {}", outcome.damage)
    } else {
        "miss".to_owned()
    };
    let down = drops_target(outcome)
        .map(|words| format!(", {target} {words}"))
        .unwrap_or_default();
    if kind_of(outcome) == Some(rpc::OutcomeKind::OpportunityAttack) {
        return format!("makes an opportunity attack on {target} \u{2014} {result}{down}");
    }
    if let Some((position, count)) = group {
        return format!(
            "makes {} attacks on {target} ({position} of {count}) \u{2014} {result}{down}",
            count_word(count)
        );
    }
    let mut text = if outcome.hit && outcome.critical {
        format!("lands a critical blow on {target} for {}", outcome.damage)
    } else if outcome.hit {
        format!("strikes at {target} \u{2014} hit for {}", outcome.damage)
    } else if outcome.grazed {
        format!("grazes {target} for {}", outcome.damage)
    } else {
        format!("swings at {target} and misses")
    };
    text.push_str(&down);
    text
}

/// The sequences at which each named creature was dropped, from the beats in resolution order:
/// `(sequence, name, down_after)` for every blow that set or cleared the down mark. The display
/// keeps this to show a down marker with the hit that dropped the creature.
pub(crate) fn down_marks(beats: &[&rpc::CombatOutcome]) -> Vec<(u64, String, bool)> {
    beats
        .iter()
        .filter_map(|outcome| match kind_of(outcome) {
            Some(rpc::OutcomeKind::Attack | rpc::OutcomeKind::OpportunityAttack) => Some((
                outcome.sequence,
                outcome.target_name.clone(),
                outcome.target_hit_points == 0 || outcome.knocked_out,
            )),
            Some(rpc::OutcomeKind::DeathSave) => Some((
                outcome.sequence,
                outcome.actor_name.clone(),
                outcome.target_hit_points == 0,
            )),
            _ => None,
        })
        .collect()
}

/// Whether `name` is shown down at `presented`: the newest recorded mark at or before it decides.
/// When blows after `presented` are still waiting, a creature with no earlier mark was not
/// down yet; with nothing waiting the newest view's own state stands (`latest`).
pub(crate) fn down_at(
    marks: &[(u64, String, bool)],
    name: &str,
    presented: u64,
    latest: bool,
) -> bool {
    let has_waiting_beats = marks
        .iter()
        .any(|(sequence, creature, _)| creature == name && *sequence > presented);
    if !has_waiting_beats {
        return latest;
    }
    marks
        .iter()
        .filter(|(sequence, creature, _)| creature == name && *sequence <= presented)
        .max_by_key(|(sequence, ..)| *sequence)
        .is_some_and(|(.., down)| *down)
}

/// A square on the grid.
pub(crate) type Square = (u32, u32);

fn adjacent(first_square: Square, second_square: Square) -> bool {
    first_square != second_square
        && first_square.0.abs_diff(second_square.0) <= 1
        && first_square.1.abs_diff(second_square.1) <= 1
}

/// A move cut in two by the reaction that interrupted it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MoveSplit {
    /// The walk up to the last square inside the reactor's reach.
    pub(crate) first: (Square, Square),
    /// The rest of the walk, after the reaction.
    pub(crate) second: (Square, Square),
}

/// Where an opportunity attack interrupts a move. SRD 5.2.1 p. 15: the attack is made when the
/// creature leaves the reactor's reach, so the move shows up to the last square in reach, then
/// the attack, then the rest. `path` is the squares entered, `start` the square it began on.
/// `None` when the mover never leaves the reactor's reach on this path (nothing to split).
pub(crate) fn split_move_at_reaction(
    start: Square,
    path: &[Square],
    reactor: Square,
) -> Option<MoveSplit> {
    let end = *path.last()?;
    let mut squares = Vec::with_capacity(path.len() + 1);
    squares.push(start);
    squares.extend_from_slice(path);
    let leaving = squares
        .windows(2)
        .position(|pair| adjacent(pair[0], reactor) && !adjacent(pair[1], reactor))?;
    let mid = squares[leaving];
    (mid != start && mid != end).then_some(MoveSplit {
        first: (start, mid),
        second: (mid, end),
    })
}

/// A move to cut in two, by the server sequences of the move and of the reaction that
/// interrupts it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SplitPlan {
    pub(crate) move_sequence: u64,
    pub(crate) reaction_sequence: u64,
    pub(crate) split: MoveSplit,
}

/// Which of an ordered beat list's moves are interrupted by a reaction, and where. A move is cut
/// by the opportunity attack that follows it directly, made by someone else on the mover.
/// `start_of` gives the square a mover began on (where the previous view left it) and `at` the
/// square a creature stands on now; a move whose squares are not known is left whole.
pub(crate) fn plan_move_splits(
    beats: &[&rpc::CombatOutcome],
    start_of: impl Fn(&str) -> Option<Square>,
    at: impl Fn(&str) -> Option<Square>,
) -> Vec<SplitPlan> {
    let mut plans = Vec::new();
    for (index, movement) in beats.iter().enumerate() {
        if kind_of(movement) != Some(rpc::OutcomeKind::Move) {
            continue;
        }
        let Some(reaction) = beats.get(index + 1).filter(|next| {
            kind_of(next) == Some(rpc::OutcomeKind::OpportunityAttack)
                && next.target_name == movement.actor_name
                && next.actor_name != movement.actor_name
        }) else {
            continue;
        };
        let path: Vec<Square> = movement
            .path
            .iter()
            .map(|cell| (cell.col, cell.row))
            .collect();
        let (Some(start), Some(reactor)) =
            (start_of(&movement.actor_name), at(&reaction.actor_name))
        else {
            continue;
        };
        if let Some(split) = split_move_at_reaction(start, &path, reactor) {
            plans.push(SplitPlan {
                move_sequence: movement.sequence,
                reaction_sequence: reaction.sequence,
                split,
            });
        }
    }
    plans
}

/// How a display that is behind plays its waiting beats.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CatchUp {
    /// On time: every beat plays at full length.
    Full,
    /// Behind: beats play shortened. A roll's die and result still show.
    Shortened,
}

/// How far behind a display is: the age of its oldest waiting beat beyond what the beats
/// queued ahead of it explain (each plays for a full beat). Waiting for one's turn in a long
/// fight step is not lag; time beyond that is.
pub(crate) fn excess_lag_ms(oldest_age_ms: u64, blows_queued: usize) -> u64 {
    let explained = u64::try_from(blows_queued)
        .unwrap_or(u64::MAX)
        .saturating_mul(crate::dice_roll::fight_beat_ms(false));
    oldest_age_ms.saturating_sub(explained)
}

/// The catch-up for a display whose queue is `lag_ms` behind (see [`excess_lag_ms`]).
pub(crate) fn catch_up_for(lag_ms: u64) -> CatchUp {
    if lag_ms > SHORTEN_LAG_MS {
        CatchUp::Shortened
    } else {
        CatchUp::Full
    }
}

/// The statuses a figure shows when its hit points and down mark are those of the beat now
/// playing rather than the newest view: the view's derived markers (bloodied, unconscious,
/// dying, stable) are replaced by the ones that follow from `hit_points`/`down` at that beat.
pub(crate) fn statuses_at_beat(
    statuses: &[df_render::battle::Status],
    hit_points: u32,
    maximum: u32,
    down: bool,
) -> Vec<df_render::battle::Status> {
    use df_render::battle::Status;
    let mut shown: Vec<Status> = statuses
        .iter()
        .copied()
        .filter(|status| {
            !matches!(
                status,
                Status::Bloodied | Status::Unconscious | Status::Dying | Status::Stable
            )
        })
        .collect();
    shown.extend(df_render::battle::derived_from_view(
        hit_points, maximum, down,
    ));
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        sequence: u64,
        kind: rpc::OutcomeKind,
        actor: &str,
        target: &str,
        hit_points_after: u32,
    ) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            sequence,
            kind: kind as i32,
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            hit: true,
            damage: 9,
            target_hit_points: hit_points_after,
            ..Default::default()
        }
    }

    fn move_to(sequence: u64, actor: &str, path: &[(u32, u32)]) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            sequence,
            kind: rpc::OutcomeKind::Move as i32,
            actor_name: actor.to_owned(),
            path: path
                .iter()
                .map(|(col, row)| rpc::GridCell {
                    col: *col,
                    row: *row,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The server's order for one hero's move: Jessa's reaction, the move, Corin's reaction
    /// that drops Brakka, then Corin's Multiattack and Jessa's own attack.
    fn evidence() -> Vec<rpc::CombatOutcome> {
        vec![
            outcome(
                5,
                rpc::OutcomeKind::OpportunityAttack,
                "Jessa",
                "Brakka",
                13,
            ),
            move_to(6, "Brakka", &[(1, 4), (1, 3)]),
            outcome(7, rpc::OutcomeKind::OpportunityAttack, "Corin", "Brakka", 0),
            outcome(8, rpc::OutcomeKind::Attack, "Corin", "Sigrun", 20),
            outcome(9, rpc::OutcomeKind::Attack, "Corin", "Sigrun", 11),
            outcome(10, rpc::OutcomeKind::Attack, "Jessa", "Sigrun", 4),
        ]
    }

    #[test]
    fn beats_come_in_the_servers_resolution_order_whatever_order_they_arrive_in() {
        let beats = evidence();
        let shuffled = [
            &beats[3], &beats[0], &beats[5], &beats[2], &beats[1], &beats[4],
        ];
        let ordered = in_resolution_order(shuffled);
        let sequences: Vec<u64> = ordered.iter().map(|beat| beat.sequence).collect();
        assert_eq!(sequences, [5, 6, 7, 8, 9, 10]);

        let twice = [&beats[1], &beats[1], &beats[0]];
        assert_eq!(in_resolution_order(twice).len(), 2, "a beat plays once");
    }

    #[test]
    fn a_reaction_hit_is_labelled_and_the_creature_that_drops_says_so() {
        let beats = evidence();
        assert_eq!(
            beat_text(&beats[0], None),
            "makes an opportunity attack on Brakka \u{2014} hit for 9"
        );
        assert_eq!(
            beat_text(&beats[2], None),
            "makes an opportunity attack on Brakka \u{2014} hit for 9, Brakka is down"
        );
        let mut knocked = outcome(1, rpc::OutcomeKind::Attack, "Corin", "Perrin", 1);
        knocked.knocked_out = true;
        assert_eq!(
            beat_text(&knocked, None),
            "strikes at Perrin \u{2014} hit for 9, Perrin falls unconscious"
        );
    }

    #[test]
    fn plain_attacks_keep_their_plain_words() {
        let hit = outcome(1, rpc::OutcomeKind::Attack, "Brakka", "Perrin", 5);
        assert_eq!(
            beat_text(&hit, None),
            "strikes at Perrin \u{2014} hit for 9"
        );
        let mut miss = hit;
        miss.hit = false;
        assert_eq!(beat_text(&miss, None), "swings at Perrin and misses");
    }

    #[test]
    fn a_multiattack_is_one_group_with_each_roll_in_its_place() {
        let beats = evidence();
        let ordered = in_resolution_order(beats.iter());
        let positions = multiattack_positions(&ordered);
        assert_eq!(
            positions,
            [None, None, None, Some((1, 2)), Some((2, 2)), None],
            "Corin's two shortsword attacks on Sigrun are one multiattack; Jessa's single attack and the reactions are not"
        );
        assert_eq!(
            beat_text(ordered[3], positions[3]),
            "makes two attacks on Sigrun (1 of 2) \u{2014} hit for 9"
        );
        assert_eq!(
            beat_text(ordered[4], positions[4]),
            "makes two attacks on Sigrun (2 of 2) \u{2014} hit for 9"
        );
    }

    #[test]
    fn two_attacks_on_different_targets_are_not_a_multiattack() {
        let beats = [
            outcome(1, rpc::OutcomeKind::Attack, "Corin", "Sigrun", 5),
            outcome(2, rpc::OutcomeKind::Attack, "Corin", "Brakka", 5),
        ];
        let ordered = in_resolution_order(beats.iter());
        assert_eq!(multiattack_positions(&ordered), [None, None]);
    }

    #[test]
    fn the_down_marker_appears_with_the_hit_that_dropped_the_creature() {
        let beats = evidence();
        let ordered = in_resolution_order(beats.iter());
        let marks = down_marks(&ordered);

        assert!(
            !down_at(&marks, "Brakka", 6, true),
            "the move: newest view says down, beat not played"
        );
        assert!(down_at(&marks, "Brakka", 7, true), "with the dropping hit");
        assert!(down_at(&marks, "Brakka", 10, true), "and after");
        assert!(
            !down_at(&marks, "Brakka", 0, true),
            "before any beat: not yet"
        );
        // A creature with no beat waiting is as the newest view says.
        assert!(down_at(&marks, "Perrin", 6, true));
        assert!(!down_at(&marks, "Perrin", 6, false));
    }

    #[test]
    fn a_creature_that_rises_is_not_shown_down() {
        let marks = vec![
            (2, "Perrin".to_owned(), true),
            (5, "Perrin".to_owned(), false),
        ];
        assert!(down_at(&marks, "Perrin", 3, false));
        assert!(!down_at(&marks, "Perrin", 5, false));
    }

    #[test]
    fn statuses_follow_the_beat_not_the_newest_view() {
        use df_render::battle::Status;
        let newest = [Status::Unconscious, Status::Poisoned];
        // Before the dropping hit: 13 of 20 hit points, up: no unconscious, no bloodied.
        let before = statuses_at_beat(&newest, 13, 20, false);
        assert!(!before.contains(&Status::Unconscious));
        assert!(before.contains(&Status::Poisoned), "conditions stay");
        // At the dropping hit: down.
        let after = statuses_at_beat(&newest, 0, 20, true);
        assert!(after.contains(&Status::Unconscious));
        // Half or less is bloodied.
        assert!(statuses_at_beat(&[], 9, 20, false).contains(&Status::Bloodied));
    }

    #[test]
    fn a_move_is_split_where_it_leaves_the_reactors_reach() {
        // Brakka starts beside Corin at (1,5) and walks (1,4), (1,3): Corin at (0,5) can reach
        // (1,5) and (1,4); stepping to (1,3) leaves it, so the attack comes after (1,4).
        let split = split_move_at_reaction((1, 5), &[(1, 4), (1, 3)], (0, 5)).expect("splits");
        assert_eq!(split.first, ((1, 5), (1, 4)));
        assert_eq!(split.second, ((1, 4), (1, 3)));
    }

    #[test]
    fn a_move_that_never_leaves_reach_or_leaves_at_once_is_not_split() {
        // Never leaves reach.
        assert_eq!(split_move_at_reaction((1, 5), &[(1, 4)], (0, 4)), None);
        // The first step leaves reach: the attack comes before any walking, nothing to split.
        assert_eq!(
            split_move_at_reaction((1, 5), &[(3, 5), (4, 5)], (0, 5)),
            None
        );
        assert_eq!(split_move_at_reaction((1, 5), &[], (0, 5)), None);
    }

    #[test]
    fn the_evidence_move_is_planned_in_two_around_the_reaction_that_follows_it() {
        let beats = evidence();
        let ordered = in_resolution_order(beats.iter());
        // Brakka began at (1,5) beside Corin (0,5); he entered (1,4) and (1,3). Jessa's reaction
        // came before the move and is not part of it; Corin's follows the move directly.
        let plans = plan_move_splits(
            &ordered,
            |name| (name == "Brakka").then_some((1, 5)),
            |name| match name {
                "Corin" => Some((0, 5)),
                "Jessa" => Some((2, 6)),
                _ => None,
            },
        );
        assert_eq!(
            plans,
            [SplitPlan {
                move_sequence: 6,
                reaction_sequence: 7,
                split: MoveSplit {
                    first: ((1, 5), (1, 4)),
                    second: ((1, 4), (1, 3)),
                }
            }]
        );
        // With the squares unknown the move stays whole.
        assert!(plan_move_splits(&ordered, |_| None, |_| None).is_empty());
    }

    #[test]
    fn lag_beyond_a_few_seconds_shortens_but_on_time_plays_in_full() {
        assert_eq!(catch_up_for(0), CatchUp::Full);
        assert_eq!(catch_up_for(SHORTEN_LAG_MS), CatchUp::Full);
        assert_eq!(catch_up_for(SHORTEN_LAG_MS + 1), CatchUp::Shortened);
    }

    #[test]
    fn waiting_for_ones_turn_in_a_long_step_is_not_lag_but_time_beyond_it_is() {
        let full = crate::dice_roll::fight_beat_ms(false);
        // Three blows queued at once: the oldest has waited two full beats' worth and is on time.
        assert_eq!(excess_lag_ms(2 * full, 3), 0);
        assert_eq!(catch_up_for(excess_lag_ms(3 * full, 3)), CatchUp::Full);
        // The same queue, but the oldest has waited five seconds longer than the beats explain.
        assert_eq!(excess_lag_ms(3 * full + 5_000, 3), 5_000);
        assert_eq!(
            catch_up_for(excess_lag_ms(3 * full + 5_000, 3)),
            CatchUp::Shortened
        );
        // An empty queue explains nothing.
        assert_eq!(excess_lag_ms(1_000, 0), 1_000);
    }
}
