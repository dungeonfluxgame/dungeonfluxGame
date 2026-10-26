//! Ordered TV completion for the native load client, using the display's timing policy.

#[expect(
    dead_code,
    reason = "native load simulation reuses shared reading/line bounds; browser machines remain unused"
)]
#[path = "stage_machines.rs"]
mod stage_machines;

use df_protocol::common as rpc;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};

// Never forget an identity and replay a line; fail rather than grow without a bound.
const LINE_IDENTITIES_CAP: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Beat,
    Closing,
}

#[derive(Default)]
pub(super) struct Presentation {
    pending: VecDeque<(u64, u64, Stage)>,
    active: Option<(u64, u64, Stage)>,
    observed: u64,
    line_seen: BTreeSet<u64>,
    closing_seen: BTreeSet<u64>,
    finished: u64,
}

#[derive(Default, Debug, PartialEq)]
pub(super) struct Progress {
    pub(super) finished: Option<u64>,
    pub(super) started: Option<(u64, u64)>,
    pub(super) closing_started: Option<(u64, u64)>,
}

impl Presentation {
    pub(super) fn observe_lines(&mut self, lines: &[rpc::TableLine]) -> io::Result<()> {
        // Narrative reservations can leave gaps and fill an earlier position later. IDs
        // deduplicate independently from combat's contiguous high-water sequence.
        let fresh: BTreeMap<_, _> = lines
            .iter()
            .filter(|line| !self.line_seen.contains(&line.sequence))
            .map(|line| (line.sequence, line))
            .collect();
        if self.line_seen.len() + fresh.len() > LINE_IDENTITIES_CAP
            || self.pending.len() + usize::from(self.active.is_some()) + fresh.len()
                > stage_machines::HELD_OUTCOMES_CAP
        {
            return Err(io::Error::other(
                "narrative identity or presentation queue overflow",
            ));
        }
        for (sequence, line) in fresh {
            let position = self
                .pending
                .iter()
                .position(|(queued, _, _)| *queued > sequence)
                .unwrap_or(self.pending.len());
            self.pending
                .insert(position, (sequence, line_hold_ms(&line.text), Stage::Beat));
            self.line_seen.insert(sequence);
        }
        Ok(())
    }

    pub(super) fn observe(&mut self, combat: &rpc::CombatView) -> io::Result<()> {
        let mut outcomes: Vec<_> = combat
            .outcomes
            .iter()
            .chain(&combat.move_outcomes)
            .chain(&combat.death_save_outcomes)
            .filter(|outcome| outcome.sequence > self.observed)
            .collect();
        outcomes.sort_by_key(|outcome| outcome.sequence);
        for outcome in outcomes {
            self.enqueue(outcome.sequence, hold_ms(outcome)?)?;
        }
        if combat.latest_sequence > self.observed {
            return Err(io::Error::other(
                "TV outcome history has a missing sequence",
            ));
        }
        let closing_hold: u64 = combat
            .closing_lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .map(|line| line_hold_ms(&line.text))
            .sum();
        if combat.finished
            && closing_hold > 0
            && !self.closing_seen.contains(&combat.latest_sequence)
        {
            if combat.latest_sequence == 0
                || self.closing_seen.len() >= stage_machines::HELD_OUTCOMES_CAP
                || self
                    .pending
                    .iter()
                    .any(|(_, _, stage)| *stage == Stage::Closing)
                || self
                    .active
                    .is_some_and(|(_, _, stage)| stage == Stage::Closing)
            {
                return Err(io::Error::other(
                    "closing presentation lacks an outcome or exceeds its bound",
                ));
            }
            self.pending
                .push_back((combat.latest_sequence, closing_hold, Stage::Closing));
            self.closing_seen.insert(combat.latest_sequence);
        }
        Ok(())
    }

    fn enqueue(&mut self, sequence: u64, hold_ms: u64) -> io::Result<()> {
        if sequence <= self.observed {
            return Ok(());
        }
        if sequence != self.observed + 1 {
            return Err(io::Error::other("TV outcome sequence is not contiguous"));
        }
        if self.pending.len() + usize::from(self.active.is_some())
            >= stage_machines::HELD_OUTCOMES_CAP
        {
            return Err(io::Error::other("TV outcome queue overflow"));
        }
        self.pending.push_back((sequence, hold_ms, Stage::Beat));
        self.observed = sequence;
        Ok(())
    }

    /// A late poll starts the next beat now; it never fast-forwards unseen presentation.
    pub(super) fn advance(&mut self, now_ms: u64) -> Progress {
        let mut progress = Progress::default();
        if let Some((sequence, due_ms, stage)) = self.active
            && now_ms >= due_ms
        {
            let closing_follows = self
                .pending
                .front()
                .is_some_and(|(next, _, kind)| *next == sequence && *kind == Stage::Closing);
            if stage == Stage::Closing || !closing_follows {
                self.finished = sequence;
                progress.finished = Some(sequence);
            }
            self.active = None;
        }
        if self.active.is_none()
            && let Some((sequence, hold_ms, stage)) = self.pending.pop_front()
        {
            self.active = Some((sequence, now_ms.saturating_add(hold_ms), stage));
            match stage {
                Stage::Beat => progress.started = Some((sequence, hold_ms)),
                Stage::Closing => progress.closing_started = Some((sequence, hold_ms)),
            }
        }
        progress
    }

    pub(super) fn finished(&self) -> u64 {
        self.finished
    }
    pub(super) fn idle(&self) -> bool {
        self.active.is_none() && self.pending.is_empty()
    }
}

fn line_hold_ms(text: &str) -> u64 {
    // The fake WAV is two seconds; shared minimum reading time is longer.
    u64::from(
        stage_machines::LINE_OPEN_MS
            + stage_machines::reading_time_ms(text)
            + stage_machines::LINE_LINGER_MS
            + stage_machines::LINE_CLOSE_MS,
    )
}

fn hold_ms(outcome: &rpc::CombatOutcome) -> io::Result<u64> {
    use df_render::battle::beat_timing;
    let animation = match rpc::OutcomeKind::try_from(outcome.kind) {
        Ok(rpc::OutcomeKind::Move) => {
            beat_timing::move_ms(u32::try_from(outcome.path.len()).map_err(io::Error::other)?)
        }
        Ok(rpc::OutcomeKind::DeathSave | rpc::OutcomeKind::Recovery) => {
            beat_timing::death_save_ms()
        }
        _ => beat_timing::blow_ms(
            outcome.attack_style == 2,
            outcome.hit || outcome.grazed,
            outcome.knocked_out,
        ),
    };
    // Conservative serial reading after the animation. Silent fake clips last two seconds,
    // shorter than the shared minimum reading time, so reading remains the bound.
    let reading: u64 = outcome
        .lines
        .iter()
        .map(|line| line_hold_ms(&line.text))
        .sum();
    Ok(animation.saturating_add(reading))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_finishes_each_beat_only_after_its_full_hold() {
        let mut tv = Presentation::default();
        tv.enqueue(1, 100).unwrap();
        tv.enqueue(2, 200).unwrap();
        assert_eq!(tv.advance(0).started, Some((1, 100)));
        assert_eq!(tv.advance(99).finished, None);
        assert_eq!(
            tv.advance(100),
            Progress {
                finished: Some(1),
                started: Some((2, 200)),
                closing_started: None
            }
        );
        assert_eq!(tv.advance(299).finished, None);
        assert_eq!(tv.advance(300).finished, Some(2));
        assert!(tv.idle());
    }

    #[test]
    fn late_polling_and_duplicate_views_never_skip_a_hold() {
        let mut tv = Presentation::default();
        tv.enqueue(1, 100).unwrap();
        tv.enqueue(1, 100).unwrap();
        tv.enqueue(2, 200).unwrap();
        tv.advance(0);
        assert_eq!(tv.advance(1000).started, Some((2, 200)));
        assert_eq!(tv.finished(), 1);
        assert_eq!(tv.advance(1199).finished, None);
        assert_eq!(tv.advance(1200).finished, Some(2));
    }

    #[test]
    fn missing_history_and_queue_overflow_fail_instead_of_releasing() {
        let mut tv = Presentation::default();
        assert!(tv.enqueue(2, 1).is_err());
        for sequence in 1..=stage_machines::HELD_OUTCOMES_CAP as u64 {
            tv.enqueue(sequence, 1).unwrap();
        }
        assert!(
            tv.enqueue(stage_machines::HELD_OUTCOMES_CAP as u64 + 1, 1)
                .is_err()
        );
        assert_eq!(tv.finished(), 0);
    }

    #[test]
    fn server_release_does_not_complete_the_simulated_tv() {
        let mut tv = Presentation::default();
        let combat = rpc::CombatView {
            latest_sequence: 1,
            released_sequence: 1,
            outcomes: vec![rpc::CombatOutcome {
                sequence: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        tv.observe(&combat).unwrap();
        tv.advance(0);
        assert_eq!(tv.finished(), 0);
        tv.observe(&combat).unwrap();
        assert!(!tv.idle());
    }
    #[test]
    fn narrative_lines_are_serial_and_duplicates_do_not_extend_the_hold() {
        let mut lines = Presentation::default();
        let log = [
            rpc::TableLine {
                sequence: 1,
                text: "A quiet moment passes.".to_owned(),
                ..Default::default()
            },
            rpc::TableLine {
                sequence: 2,
                text: "The heroes wait.".to_owned(),
                ..Default::default()
            },
        ];
        let hold = line_hold_ms(&log[0].text);
        lines.observe_lines(&log).unwrap();
        assert_eq!(lines.advance(0).started, Some((1, hold)));
        lines.observe_lines(&log).unwrap();
        assert_eq!(lines.advance(hold - 1).finished, None);
        assert_eq!(lines.advance(hold).finished, Some(1));
        assert!(!lines.idle());
        assert_eq!(
            lines.advance(hold + line_hold_ms(&log[1].text)).finished,
            Some(2)
        );
        assert!(lines.idle());
    }
    #[test]
    fn sparse_narrative_ids_are_read_without_inventing_missing_lines() {
        let mut tv = Presentation::default();
        let lines: Vec<_> = [3, 7]
            .into_iter()
            .map(|sequence| rpc::TableLine {
                sequence,
                text: "Wait.".to_owned(),
                ..Default::default()
            })
            .collect();
        let hold = line_hold_ms("Wait.");
        tv.observe_lines(&lines).unwrap();
        assert_eq!(tv.advance(0).started, Some((3, hold)));
        assert_eq!(tv.advance(hold).started, Some((7, hold)));
        assert_eq!(tv.advance(hold * 2).finished, Some(7));
        tv.observe_lines(&lines).unwrap();
        assert!(tv.idle());
    }

    #[test]
    fn late_narrative_fill_is_read_once_after_the_active_line_finishes() {
        let mut tv = Presentation::default();
        let line = |sequence| rpc::TableLine {
            sequence,
            text: "Wait.".to_owned(),
            ..Default::default()
        };
        let hold = line_hold_ms("Wait.");
        tv.observe_lines(&[line(3), line(9)]).unwrap();
        tv.advance(0);
        tv.observe_lines(&[line(2), line(3), line(9), line(2)])
            .unwrap();
        assert_eq!(tv.advance(hold - 1).finished, None);
        assert_eq!(tv.advance(hold).started, Some((2, hold)));
        assert_eq!(tv.advance(hold * 2).started, Some((9, hold)));
        tv.advance(hold * 3);
        tv.observe_lines(&[line(1), line(2), line(3), line(9)])
            .unwrap();
        assert_eq!(tv.advance(hold * 3).started, Some((1, hold)));
        tv.advance(hold * 4);
        tv.observe_lines(&[line(1), line(2), line(3), line(9)])
            .unwrap();
        assert!(tv.idle());
    }

    #[test]
    fn a_narrative_burst_over_the_queue_bound_fails_without_dropping_lines() {
        let mut tv = Presentation::default();
        let lines: Vec<_> = (1..=stage_machines::HELD_OUTCOMES_CAP as u64 + 1)
            .map(|sequence| rpc::TableLine {
                sequence,
                ..Default::default()
            })
            .collect();
        assert!(tv.observe_lines(&lines).is_err());
        assert!(tv.idle());
        tv.observe_lines(&lines[..1]).unwrap();
        assert_eq!(tv.advance(0).started.map(|(sequence, _)| sequence), Some(1));
    }
    #[test]
    fn final_ack_waits_for_closing_reading_and_repeated_views_never_restart_it() {
        let mut tv = Presentation::default();
        tv.enqueue(1, 100).unwrap();
        let combat = rpc::CombatView {
            latest_sequence: 1,
            finished: true,
            closing_lines: vec![rpc::FightLine {
                text: "Wait.".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let hold = line_hold_ms("Wait.");
        tv.observe(&combat).unwrap();
        tv.advance(0);
        assert_eq!(tv.advance(99).finished, None);
        let progress = tv.advance(100);
        assert_eq!(progress.finished, None);
        assert_eq!(progress.closing_started, Some((1, hold)));
        assert_eq!(tv.finished(), 0);
        tv.observe(&combat).unwrap();
        assert_eq!(tv.advance(100 + hold - 1).finished, None);
        assert_eq!(tv.advance(100 + hold).finished, Some(1));
        tv.observe(&combat).unwrap();
        assert!(tv.idle());
    }

    #[test]
    fn a_late_closing_line_holds_input_and_reuses_only_the_existing_sequence() {
        let mut tv = Presentation::default();
        tv.enqueue(1, 100).unwrap();
        tv.advance(0);
        tv.advance(100);
        let combat = rpc::CombatView {
            latest_sequence: 1,
            finished: true,
            closing_lines: vec![rpc::FightLine {
                text: "Wait.".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        tv.observe(&combat).unwrap();
        assert!(!tv.idle());
        assert_eq!(
            tv.advance(100).closing_started,
            Some((1, line_hold_ms("Wait.")))
        );
        assert_eq!(tv.advance(100 + line_hold_ms("Wait.")).finished, Some(1));
        tv.observe(&combat).unwrap();
        assert!(tv.idle());
        let mut empty = Presentation::default();
        assert!(
            empty
                .observe(&rpc::CombatView {
                    latest_sequence: 0,
                    ..combat
                })
                .is_err()
        );
    }
    #[test]
    fn closing_has_its_own_slot_after_a_full_outcome_batch() {
        let mut tv = Presentation::default();
        for sequence in 1..=stage_machines::HELD_OUTCOMES_CAP as u64 {
            tv.enqueue(sequence, 1).unwrap();
        }
        let last = stage_machines::HELD_OUTCOMES_CAP as u64;
        tv.observe(&rpc::CombatView {
            latest_sequence: last,
            finished: true,
            closing_lines: vec![rpc::FightLine {
                text: "Wait.".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        tv.advance(0);
        for now in 1..last {
            assert_eq!(tv.advance(now).finished, Some(now));
        }
        let progress = tv.advance(last);
        assert_eq!(progress.finished, None);
        assert_eq!(
            progress.closing_started,
            Some((last, line_hold_ms("Wait.")))
        );
        assert_eq!(
            tv.advance(last + line_hold_ms("Wait.")).finished,
            Some(last)
        );
    }
}
